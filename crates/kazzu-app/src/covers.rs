//! Capas de provedores remotos: baixa só o que a tela mostra, guarda em disco e decodifica fora da thread de UI.
//!
//! Privacidade: só `https`, só de hosts de imagem do YouTube/Google, só quando o usuário vê o item (o controller pede
//! as capas de uma tela que ele abriu). Nada é pré-carregado em massa e nenhum identificador é enviado.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::RgbaImage;

use crate::art::{self, Pixels};

/// Lado máximo das capas de lista e cartão (a tela nunca as mostra maiores que 320 px lógicos).
pub const LIST_SIDE: u32 = if cfg!(target_os = "android") { 192 } else { 320 };
/// Lado máximo da capa do player.
pub const PLAYER_SIDE: u32 = 640;
const MAX_BYTES: u64 = 6 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
/// Quantos downloads simultâneos um pedido de capas faz.
const PARALLEL: usize = 4;
/// Maior lado e memória aceitos ao decodificar uma capa remota.
const MAX_SIDE_PX: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
/// Teto do cache de capas em disco e de quantas gravações entre uma limpeza e outra.
const CACHE_MAX_BYTES: u64 = 128 * 1024 * 1024;
const TRIM_EVERY: u32 = 16;

/// Hosts de imagem aceitos (miniaturas do YouTube Music).
fn host_allowed(host: &str) -> bool {
    host == "music.youtube.com" || host == "i.ytimg.com" || host.ends_with(".ytimg.com") || host.ends_with(".googleusercontent.com") || host.ends_with(".ggpht.com")
}

/// `https://host/...` -> `host`, se o esquema for https.
fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.split('@').next_back()?.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

pub fn url_allowed(url: &str) -> bool {
    https_host(url).is_some_and(host_allowed)
}

/// FNV-1a de 64 bits: estável entre versões do Rust (o cache em disco sobrevive a atualizações).
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Origem dos bytes de uma imagem. Em testes entra uma implementação sem rede.
pub trait ImageSource: Send + Sync {
    fn get(&self, url: &str) -> Option<Vec<u8>>;
}

/// HTTP real (ureq + rustls). **Não segue redirecionamentos** e (por padrão) recusa `http`: a lista de hosts
/// permitidos só vale para a URL inicial, então um redirecionamento para fora dela seria um buraco. Só aceita
/// resposta com `Content-Type: image/*`.
pub struct HttpImages {
    agent: ureq::Agent,
}

impl HttpImages {
    /// `https_only = false` existe só para testar contra um servidor local.
    pub fn new(https_only: bool) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(true)
            .max_redirects(0)
            .https_only(https_only)
            .build()
            .into();
        Self { agent }
    }
}

impl Default for HttpImages {
    fn default() -> Self {
        Self::new(true)
    }
}

impl ImageSource for HttpImages {
    fn get(&self, url: &str) -> Option<Vec<u8>> {
        let mut resp = self.agent.get(url).call().ok()?;
        let is_image = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|ct| ct.trim().to_ascii_lowercase().starts_with("image/"));
        if !is_image {
            log::warn!("capa ignorada: a resposta não é uma imagem");
            return None;
        }
        let mut buf = Vec::new();
        resp.body_mut().as_reader().take(MAX_BYTES).read_to_end(&mut buf).ok()?;
        Some(buf)
    }
}

/// Decodifica com limites (4096×4096 e 64 MiB de memória): uma imagem hostil ou gigante não derruba o app (Android).
fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE_PX);
    limits.max_image_height = Some(MAX_SIDE_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok().map(|i| i.to_rgba8())
}

/// Cache em disco (`<dir>/<hash>.img`, bytes originais que decodificam) + origem dos bytes. O diretório tem teto:
/// passando dele, os arquivos mais antigos saem primeiro.
pub struct CoverFetcher {
    dir: Option<PathBuf>,
    source: Box<dyn ImageSource>,
    max_bytes: u64,
    writes: std::sync::atomic::AtomicU32,
}

impl CoverFetcher {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self::with_source(dir, Box::new(HttpImages::default()))
    }

    pub fn with_source(dir: Option<PathBuf>, source: Box<dyn ImageSource>) -> Self {
        Self { dir, source, max_bytes: CACHE_MAX_BYTES, writes: std::sync::atomic::AtomicU32::new(0) }
    }

    /// Teto do diretório de capas (padrão 128 MiB).
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    fn path(&self, url: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{:016x}.img", fnv1a(url))))
    }

    /// Imagem decodificada (cheia): disco primeiro; se o arquivo do cache não decodifica, é apagado e busca de novo.
    fn decoded(&self, url: &str) -> Option<RgbaImage> {
        let path = self.path(url);
        if let Some(p) = &path
            && let Ok(bytes) = std::fs::read(p)
        {
            match decode(&bytes) {
                Some(img) => return Some(img),
                None => {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        if !url_allowed(url) {
            log::warn!("capa ignorada (host não permitido): {}", https_host(url).unwrap_or("?"));
            return None;
        }
        let bytes = self.source.get(url)?;
        // só entra no cache o que decodifica (um HTML de erro ou imagem corrompida nunca fica gravado)
        let img = decode(&bytes)?;
        if let Some(p) = path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
            if self.writes.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(TRIM_EVERY) {
                self.trim();
            }
        }
        Some(img)
    }

    /// Mantém o diretório de capas abaixo do teto apagando os arquivos mais antigos (até 80% do teto).
    pub fn trim(&self) {
        let Some(dir) = &self.dir else { return };
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
            .filter_map(Result::ok)
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|f| f.0);
        let goal = self.max_bytes / 10 * 8;
        for (_, len, path) in files {
            if total <= goal {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    /// Imagem decodificada, já reduzida a `max_side`. `None` se não deu (rede, formato, host).
    pub fn image(&self, url: &str, max_side: u32) -> Option<RgbaImage> {
        self.decoded(url).map(|img| art::fit_within(img, max_side))
    }

    pub fn pixels(&self, url: &str) -> Option<Pixels> {
        self.image(url, LIST_SIDE).map(|i| art::to_pixels(&i))
    }

    /// Baixa várias capas (até `PARALLEL` ao mesmo tempo) e entrega cada uma assim que fica pronta.
    pub fn fetch_many(&self, urls: &[String], deliver: &(dyn Fn(String, Pixels) + Sync)) {
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL.min(urls.len()) {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(url) = urls.get(i) else { break };
                        if let Some(px) = self.pixels(url) {
                            deliver(url.clone(), px);
                        }
                    }
                });
            }
        });
    }

    pub fn cache_dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        calls: AtomicUsize,
        bytes: Vec<u8>,
    }

    impl ImageSource for std::sync::Arc<Fake> {
        fn get(&self, _url: &str) -> Option<Vec<u8>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Some(self.bytes.clone())
        }
    }

    fn png() -> Vec<u8> {
        let img = RgbaImage::from_pixel(400, 300, image::Rgba([200, 80, 40, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn only_https_image_hosts_are_allowed() {
        assert!(url_allowed("https://lh3.googleusercontent.com/abc=w544-h544"));
        assert!(url_allowed("https://i.ytimg.com/vi/x/hqdefault.jpg"));
        assert!(url_allowed("https://yt3.ggpht.com/x"));
        assert!(url_allowed("https://music.youtube.com/img/x.png"));
        assert!(!url_allowed("https://www.youtube.com/x"));
        assert!(!url_allowed("http://i.ytimg.com/x"));
        assert!(!url_allowed("https://evil.example.com/x"));
        assert!(!url_allowed("https://i.ytimg.com.evil.com/x"));
        assert!(!url_allowed("file:///etc/passwd"));
        assert!(!url_allowed("https://user@evil.com/x"));
    }

    #[test]
    fn images_are_cached_on_disk_and_shrunk() {
        let dir = tempfile::tempdir().unwrap();
        let fake = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: png() });
        let f = CoverFetcher::with_source(Some(dir.path().to_path_buf()), Box::new(fake.clone()));
        let url = "https://lh3.googleusercontent.com/a=w544-h544";
        let img = f.image(url, 200).unwrap();
        assert_eq!(img.dimensions(), (200, 150));
        // segunda vez: vem do disco, sem rede
        assert!(f.image(url, 200).is_some());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        // e sobrevive a uma nova instância (reinício do app)
        let again = CoverFetcher::with_source(Some(dir.path().to_path_buf()), Box::new(fake.clone()));
        assert!(again.image(url, 200).is_some());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn blocked_hosts_never_reach_the_network() {
        let fake = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: png() });
        let f = CoverFetcher::with_source(None, Box::new(fake.clone()));
        assert!(f.image("https://evil.example.com/a.png", 100).is_none());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fetch_many_delivers_every_cover() {
        let fake = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: png() });
        let f = CoverFetcher::with_source(None, Box::new(fake));
        let urls: Vec<String> = (0..9).map(|i| format!("https://i.ytimg.com/vi/{i}/hq.jpg")).collect();
        let got = Mutex::new(Vec::new());
        f.fetch_many(&urls, &|u, _| got.lock().unwrap().push(u));
        let mut got = got.into_inner().unwrap();
        got.sort();
        let mut want = urls;
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn bytes_that_do_not_decode_are_never_cached_and_a_corrupt_cache_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let bad = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: b"<html>erro</html>".to_vec() });
        let f = CoverFetcher::with_source(Some(dir.path().to_path_buf()), Box::new(bad.clone()));
        let url = "https://i.ytimg.com/vi/x/hq.jpg";
        assert!(f.image(url, 100).is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nada gravado");
        // arquivo corrompido já no cache: é apagado e a rede é consultada de novo
        let good = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: png() });
        let f = CoverFetcher::with_source(Some(dir.path().to_path_buf()), Box::new(good.clone()));
        std::fs::write(dir.path().join(format!("{:016x}.img", fnv1a(url))), b"lixo").unwrap();
        assert!(f.image(url, 100).is_some());
        assert_eq!(good.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn huge_images_are_refused_before_allocating() {
        // 20000×20000 px: o cabeçalho PNG diz o tamanho; o limite de 4096 recusa sem alocar ~1,6 GB
        let img = image::GrayImage::from_pixel(20_000, 8, image::Luma([7]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        assert!(decode(&out.into_inner()).is_none());
        assert!(decode(&png()).is_some());
    }

    #[test]
    fn the_cover_directory_has_a_cap_and_drops_the_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let fake = std::sync::Arc::new(Fake { calls: AtomicUsize::new(0), bytes: png() });
        let size = png().len() as u64;
        let f = CoverFetcher::with_source(Some(dir.path().to_path_buf()), Box::new(fake)).with_max_bytes(size * 5);
        for i in 0..40 {
            f.image(&format!("https://i.ytimg.com/vi/{i}/hq.jpg"), 50).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        f.trim();
        let total: u64 = std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
        assert!(total <= size * 5, "{total} > {}", size * 5);
        // a mais nova sobreviveu
        assert!(dir.path().join(format!("{:016x}.img", fnv1a("https://i.ytimg.com/vi/39/hq.jpg"))).exists());
    }

    /// Servidor HTTP mínimo: devolve `resp` a cada conexão e conta quantas houve.
    fn serve(resp: String) -> (u16, std::sync::Arc<AtomicUsize>) {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                h.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (port, hits)
    }

    #[test]
    fn redirects_are_not_followed_and_non_images_are_refused() {
        // o destino do redirecionamento NUNCA pode ser consultado, mesmo que o host inicial seja permitido
        let (victim_port, victim_hits) = serve("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx".into());
        let (port, hits) = serve(format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{victim_port}/x.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
        let http = HttpImages::new(false);
        assert!(http.get(&format!("http://127.0.0.1:{port}/a.png")).is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(victim_hits.load(Ordering::SeqCst), 0, "seguiu o redirecionamento");
        // resposta 200 que não é imagem
        let (p2, _) = serve("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi".into());
        assert!(http.get(&format!("http://127.0.0.1:{p2}/")).is_none());
        // e uma imagem de verdade passa
        let body = png();
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let mut resp = head.into_bytes();
        resp.extend(body);
        let (p3, _) = serve_bytes(resp);
        assert!(http.get(&format!("http://127.0.0.1:{p3}/")).is_some());
        // por padrão `http` nem é tentado
        assert!(HttpImages::default().get(&format!("http://127.0.0.1:{p3}/")).is_none());
    }

    fn serve_bytes(resp: Vec<u8>) -> (u16, std::sync::Arc<AtomicUsize>) {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                h.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(&resp);
            }
        });
        (port, hits)
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
