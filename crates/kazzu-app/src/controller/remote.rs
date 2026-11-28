//! Parte do controller que fala com o provedor remoto (YouTube Music): Início, Explorar, busca, detalhes, capas,
//! a fila "sob demanda" com rádio, URL vencida e curtidas remotas. Ver `crate::remote` para o trabalho de rede.
//!
//! **Fila sob demanda (contexto).** Resolver o stream de cada faixa de uma playlist de 200 músicas seria rede
//! demais (e o YouTube estrangula pedidos em massa). Então o motor só recebe a faixa pedida e mais duas à frente
//! (`LOOKAHEAD`); o resto da lista fica em `Context::tail` e entra quando a fila do motor encolhe. Acabou a lista e
//! "Continuar com músicas parecidas" está ligado: o controller pede o rádio da última faixa e segue em frente.
//! Listas só de arquivos locais continuam inteiras no motor (sem rede, nada a economizar).

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use kazzu_audio::StreamEvent;
use kazzu_core::{QueueItemId, Track as CoreTrack};
use kazzu_ytm::Radio;

use super::*;
use crate::vm::CoverSlots;
use crate::catalog::ErrorKind;
use crate::remote::{RemoteJob, RemoteResult};

/// Quantas faixas já resolvidas o motor deve ter à frente da atual.
const LOOKAHEAD: usize = 2;
/// Quantas faixas do rádio entram por vez.
const RADIO_BATCH: usize = 25;

/// Uma entrada da fila gerenciada: arquivo local (por id) ou faixa remota (por referência).
#[derive(Clone, Debug)]
pub enum Entry {
    Local(String),
    Remote(Box<CoreTrack>),
}

impl Entry {
    /// Id como a UI o conhece (`"123"` ou `"yt:…"`).
    pub fn id(&self) -> String {
        match self {
            Entry::Local(id) => id.clone(),
            Entry::Remote(t) => ytm_vm::remote_id(t.id.as_str()),
        }
    }
}

pub(super) enum ToastAction {
    None,
    UndoLike { id: String, was_liked: bool },
    UndoQueue(Vec<QueueItemId>),
    RetryPlay,
    RetryStart,
    RetryLookahead,
    /// A URL renovada também foi recusada: tenta de novo por ação do usuário.
    RetryReresolve(QueueItemId),
}

/// Onde uma linha da fila está.
enum RowPos {
    Engine(usize),
    Tail(usize),
}

/// URL vencida: no máximo `EXPIRED_MAX` renovações automáticas por faixa dentro de `EXPIRED_WINDOW`.
const EXPIRED_MAX: usize = 2;
const EXPIRED_WINDOW: Duration = Duration::from_secs(60);

pub(super) struct Context {
    pub seq: u64,
    /// Entradas ainda não entregues ao motor, na ordem de reprodução.
    pub tail: VecDeque<Entry>,
    /// A lista como foi pedida (repetir tudo, desembaralhar).
    pub all: Vec<Entry>,
    pub shuffled: bool,
    pub repeat_all: bool,
    /// A faixa pedida ainda não chegou ao motor.
    pub start_pending: bool,
    pub start: Entry,
    /// Há uma resolução de "próxima" em andamento.
    pub resolving: bool,
    /// A rede falhou ao resolver a próxima: espera o usuário tentar de novo.
    pub stalled: bool,
    pub radio_seed: Option<String>,
    pub radio: Option<Box<Radio>>,
    pub radio_fetching: bool,
    pub radio_exhausted: bool,
    pub radio_empty_rounds: u8,
    /// O motor parou no fim da fila esperando uma próxima que ainda estava a caminho: toca-a quando chegar.
    pub resume: bool,
    /// Falhas de rede seguidas (recuo exponencial) e quando tentar de novo.
    pub fail_count: u32,
    pub retry_after: Option<Instant>,
    /// Ids (da UI) já tocados, para o rádio não repetir.
    pub played: HashSet<String>,
}

/// Radio bookkeeping. Its pending request holds no ordered track entries.
pub(super) struct EngineRadio {
    seq: u64,
    first: QueueItemId,
    seed: String,
    continuation: Option<Box<Radio>>,
    fetching: bool,
    exhausted: bool,
    empty_rounds: u8,
    stopped: bool,
    retry_after: Option<Instant>,
}

/// Embaralhamento simples (xorshift semeado pelo relógio): suficiente para ordem de fila, sem dependência nova.
fn shuffle<T>(v: &mut [T]) {
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

impl<V: View> Controller<V> {
    // ----------------------------------------------------------------------------------------
    // Estado compartilhado
    // ----------------------------------------------------------------------------------------

    pub(super) fn remember_remote(&mut self, t: &CoreTrack) {
        if matches!(t.source, TrackSource::Remote { .. }) {
            self.remote_tracks.insert(ytm_vm::track_ui_id(t), t.clone());
        }
    }

    fn remember_all(&mut self, tracks: &[CoreTrack]) {
        for t in tracks {
            self.remember_remote(t);
        }
    }

    pub(super) fn set_online(&mut self, online: bool) {
        if online == self.online {
            return;
        }
        self.online = online;
        self.view.set_online(online);
        if online {
            let retry = self.remote_enabled && (matches!(self.home.state, LoadStateVm::Error | LoadStateVm::Offline) || self.home.stale);
            // a conexão voltou: o aviso "Tentar de novo" ficou sem sentido; a fila parada e a faixa travada retomam
            self.toast("Conexão de volta", false);
            if self.stalled {
                self.stalled = false;
                self.pb.error.clear();
                self.view.set_playback(&self.pb);
                self.engine.play();
            }
            self.unstall(true);
            if retry {
                self.retry_home();
            }
        }
    }

    /// Preenche as capas que já estão em memória e pede as que faltam (uma vez cada).
    pub(super) fn cover_pass(&mut self, vm: &mut dyn CoverSlots) {
        if !self.remote_enabled {
            return;
        }
        let covers = &mut self.covers;
        apply_covers(vm, &mut |url| covers.get(&url.to_owned()).cloned());
        let now = Instant::now();
        let (requested, retry) = (&self.covers_requested, self.cover_retry);
        let need: Vec<String> = missing_covers(vm).into_iter().filter(|u| requested.get(u).is_none_or(|t| now.duration_since(*t) >= retry)).collect();
        if !need.is_empty() {
            for u in &need {
                self.covers_requested.insert(u.clone(), now);
            }
            self.jobs.submit(Job::Remote(RemoteJob::Covers { urls: need }));
        }
    }

    fn quality(&self) -> kazzu_core::StreamQuality {
        self.cfg.stream_quality.to_core()
    }

    // ----------------------------------------------------------------------------------------
    // Início e Explorar
    // ----------------------------------------------------------------------------------------

    pub(super) fn request_home(&mut self, section: HomeSectionVm) {
        self.home_seq += 1;
        let recents = std::mem::take(&mut self.home.recents);
        self.home = HomeVm { state: LoadStateVm::Loading, section, source_label: ytm_vm::SOURCE_LABEL.into(), recents, ..Default::default() };
        self.jobs.submit(Job::Remote(RemoteJob::LoadHome { seq: self.home_seq, section }));
        self.publish_home();
    }

    pub fn retry_home(&mut self) {
        if self.remote_enabled {
            self.request_home(self.home.section);
        }
    }

    pub fn home_section_changed(&mut self, section: HomeSectionVm) {
        if self.remote_enabled && section != self.home.section {
            self.request_home(section);
        }
    }

    fn home_has_content(&self) -> bool {
        !(self.home.highlights.is_empty() && self.home.quick_picks.is_empty() && self.home.mixes.is_empty() && self.home.albums.is_empty())
    }

    pub(super) fn publish_home(&mut self) {
        let mut vm = self.home.clone();
        vm.banner = self.remote_enabled && !self.cfg.first_run_dismissed;
        if vm.source_label.is_empty() && self.remote_enabled {
            vm.source_label = ytm_vm::SOURCE_LABEL.into();
        }
        self.flag_tracks(&mut vm.quick_picks);
        self.flag_tracks(&mut vm.recents);
        self.cover_pass(&mut vm);
        self.view.set_home(&vm);
    }

    pub(super) fn request_explore(&mut self) {
        self.explore_requested = true;
        self.explore_seq += 1;
        self.explore = ExploreVm { state: LoadStateVm::Loading, ..Default::default() };
        self.jobs.submit(Job::Remote(RemoteJob::LoadExplore { seq: self.explore_seq }));
        self.publish_explore();
    }

    pub fn retry_explore(&mut self) {
        if self.remote_enabled {
            self.request_explore();
        }
    }

    pub(super) fn publish_explore(&mut self) {
        let mut vm = self.explore.clone();
        self.flag_tracks(&mut vm.trending);
        self.cover_pass(&mut vm);
        self.view.set_explore(&vm);
    }

    // ----------------------------------------------------------------------------------------
    // Resultados de rede
    // ----------------------------------------------------------------------------------------

    pub(super) fn on_remote(&mut self, r: RemoteResult) {
        match r {
            RemoteResult::Home { seq, vm, tracks, cached } => {
                if seq != self.home_seq {
                    return;
                }
                if !cached {
                    self.set_online(true);
                }
                self.remember_all(&tracks);
                let recents = std::mem::take(&mut self.home.recents);
                self.home = *vm;
                self.home.recents = recents;
                self.publish_home();
            }
            RemoteResult::HomeFailed { seq, kind, message } => {
                if seq != self.home_seq {
                    return;
                }
                log::warn!("início: {message}");
                if ytm_vm::is_offline(kind) {
                    self.set_online(false);
                }
                let (title, text, _) = ytm_vm::error_copy(kind);
                if self.home_has_content() {
                    // erro com conteúdo em cache: a tela continua e ganha uma faixa de aviso
                    self.home.stale = true;
                } else {
                    self.home.state = if ytm_vm::is_offline(kind) { LoadStateVm::Offline } else { LoadStateVm::Error };
                }
                self.home.error_title = title.into();
                self.home.error_text = text.into();
                self.home.error_code = ytm_vm::error_code(kind);
                self.publish_home();
            }
            RemoteResult::Explore { seq, vm, tracks, categories } => {
                if seq != self.explore_seq {
                    return;
                }
                self.set_online(true);
                self.remember_all(&tracks);
                self.categories = categories;
                self.explore = *vm;
                self.publish_explore();
            }
            RemoteResult::ExploreFailed { seq, kind } => {
                if seq != self.explore_seq {
                    return;
                }
                if ytm_vm::is_offline(kind) {
                    self.set_online(false);
                }
                let (_, text, _) = ytm_vm::error_copy(kind);
                self.explore.state = if ytm_vm::is_offline(kind) { LoadStateVm::Offline } else { LoadStateVm::Error };
                self.explore.error_text = if ytm_vm::is_offline(kind) { "Explorar precisa de internet.".into() } else { text.into() };
                self.publish_explore();
            }
            RemoteResult::Search { seq, vm, tracks, .. } => {
                if seq != self.search_seq {
                    return;
                }
                self.set_online(true);
                self.remember_all(&tracks);
                self.search_remote = Some(*vm);
                self.publish_search();
            }
            RemoteResult::SearchFailed { seq, kind } => {
                if seq != self.search_seq {
                    return;
                }
                if ytm_vm::is_offline(kind) {
                    self.set_online(false);
                }
                self.search_remote_failed = Some(kind);
                self.publish_search();
            }
            RemoteResult::Suggestions { seq, list } => {
                if seq == self.search_seq {
                    self.search_suggestions = list;
                    self.publish_search();
                }
            }
            RemoteResult::Detail { seq, vm, tracks, what, album } => {
                if seq != self.detail_seq {
                    return;
                }
                self.set_online(true);
                self.remember_all(&tracks);
                let mut vm = *vm;
                self.flag_tracks(&mut vm.tracks);
                self.detail_ids = vm.tracks.iter().map(|t| t.id.clone()).collect();
                self.detail_title = vm.title.clone();
                self.detail_what = Some(what);
                self.detail_album = album;
                self.show_detail(vm);
                self.restore_detail_scroll();
            }
            RemoteResult::DetailFailed { seq, kind } => {
                if seq != self.detail_seq {
                    return;
                }
                if ytm_vm::is_offline(kind) {
                    self.set_online(false);
                }
                let (_, text, _) = ytm_vm::error_copy(kind);
                let vm = DetailVm { state: if ytm_vm::is_offline(kind) { LoadStateVm::Offline } else { LoadStateVm::Error }, error_text: text.into(), title: self.detail_vm.title.clone(), ..Default::default() };
                self.show_detail(vm);
            }
            RemoteResult::PlayTracks { ctx_seq, tracks, label, shuffle } => {
                if ctx_seq != self.ctx_seq {
                    return;
                }
                self.remember_all(&tracks);
                let entries: Vec<Entry> = tracks.into_iter().map(|t| Entry::Remote(Box::new(t))).collect();
                self.start_context(entries, 0, &label, shuffle);
            }
            RemoteResult::PlayFailed { ctx_seq, kind } => {
                if ctx_seq == self.ctx_seq {
                    let (title, text, _) = ytm_vm::error_copy(kind);
                    self.toast(&format!("{title}. {text}"), true);
                }
            }
            RemoteResult::Resolved { ctx_seq, item, quality_label, for_start } => self.on_resolved(ctx_seq, *item, quality_label, for_start),
            RemoteResult::ResolveFailed { ctx_seq, track, kind, for_start } => self.on_resolve_failed(ctx_seq, *track, kind, for_start),
            RemoteResult::Queued { item, quality_label, play_next } => {
                self.set_online(true);
                if !quality_label.is_empty() {
                    self.quality_labels.insert(item.id, quality_label);
                }
                self.queued(vec![*item], play_next);
            }
            RemoteResult::QueueFailed { kind } => {
                let (title, text, _) = ytm_vm::error_copy(kind);
                self.toast(&format!("{title}. {text}"), true);
            }
            RemoteResult::Replaced { id, source, quality_label } => {
                self.set_online(true);
                self.engine.replace_stream(id, source);
                if !quality_label.is_empty() {
                    if self.current.and_then(|c| self.queue_items.get(c)).is_some_and(|i| i.id == id) {
                        self.pb.quality_label = quality_label.clone();
                        self.view.set_playback(&self.pb);
                    }
                    self.quality_labels.insert(id, quality_label);
                }
            }
            RemoteResult::ReplaceFailed { kind, .. } => {
                if ytm_vm::is_offline(kind) {
                    self.set_online(false);
                }
                let (title, text, retry) = ytm_vm::error_copy(kind);
                let msg = format!("{title}. {text}");
                if retry {
                    self.stalled = true;
                    self.toast_with(&msg, true, "Tentar de novo", ToastAction::RetryPlay);
                } else {
                    self.toast(&msg, true);
                }
            }
            RemoteResult::Radio { ctx_seq, tracks, radio } => self.on_radio(ctx_seq, tracks, *radio),
            RemoteResult::RadioFailed { ctx_seq, kind } => self.on_radio_failed(ctx_seq, kind),
            RemoteResult::Covers(list) => {
                let urls: Vec<String> = list.iter().map(|(u, _)| u.clone()).collect();
                for (url, px) in list {
                    self.covers.insert(url, px);
                }
                self.republish_covered(&urls);
            }
        }
    }

    /// Capas novas chegaram: refaz o que está na tela com elas.
    /// Capas novas chegaram: refaz só o que na tela USA essas URLs, e só a tela à vista (as outras ficam marcadas e são
    /// refeitas ao navegar até elas). Antes cada lote de 4 capas refazia Início, Busca, Detalhe e Fila inteiros.
    fn republish_covered(&mut self, urls: &[String]) {
        fn touches(vm: &mut dyn CoverSlots, urls: &HashSet<&str>) -> bool {
            let mut hit = false;
            vm.each_cover(&mut |u, _| hit |= u.is_some_and(|u| urls.contains(u)));
            hit
        }
        let set: HashSet<&str> = urls.iter().map(String::as_str).collect();
        if touches(&mut self.home, &set) {
            if self.tab == NavTab::Home {
                self.publish_home();
            } else {
                self.stale_screens.insert(NavTab::Home);
            }
        }
        if self.explore_requested && touches(&mut self.explore, &set) {
            if matches!(self.tab, NavTab::Home | NavTab::Explore | NavTab::Search) {
                self.publish_explore();
            } else {
                self.stale_screens.insert(NavTab::Explore);
            }
        }
        if self.search_remote.as_mut().is_some_and(|r| touches(r, &set)) {
            if self.tab == NavTab::Search {
                self.publish_search();
            } else {
                self.stale_screens.insert(NavTab::Search);
            }
        }
        if self.detail_open && touches(&mut self.detail_vm, &set) {
            let vm = self.detail_vm.clone();
            self.show_detail(vm);
        }
        if touches(&mut self.queue_vm, &set) {
            self.publish_queue();
        }
    }

    pub(super) fn show_detail(&mut self, mut vm: DetailVm) {
        self.cover_pass(&mut vm);
        self.detail_vm = vm.clone();
        self.view.set_detail(&vm);
        self.view.open_detail(true);
        self.detail_open = true;
    }

    // ----------------------------------------------------------------------------------------
    // Abrir e tocar itens do catálogo
    // ----------------------------------------------------------------------------------------

    pub fn open_album(&mut self, id: &str) {
        match ytm_vm::strip_remote(id) {
            Some(raw) => self.open_remote(RemoteRef::album_or_playlist(raw)),
            None => self.request_detail(DetailRef::Album(id.to_owned())),
        }
    }

    pub fn open_playlist(&mut self, id: &str) {
        match ytm_vm::strip_remote(id) {
            Some(raw) => self.open_remote(RemoteRef::album_or_playlist(raw)),
            None => self.request_detail(DetailRef::Playlist(id.to_owned())),
        }
    }

    /// `kind` e `id` de um cartão (`Highlight`): álbum, playlist, artista, humor, faixa, curtidas, embaralhar.
    pub fn open_item(&mut self, kind: &str, id: &str) {
        match kind {
            "album" => self.open_album(id),
            "playlist" => self.open_playlist(id),
            "artist" => match ytm_vm::strip_remote(id) {
                Some(raw) => self.open_remote(RemoteRef::Artist(raw.to_owned())),
                None => self.request_detail(DetailRef::Artist(id.to_owned())),
            },
            "mood" => {
                if let Some(c) = id.strip_prefix("mood:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| self.categories.get(i)).cloned() {
                    self.open_remote(RemoteRef::Browse { title: c.title, target: c.browse });
                }
            }
            "liked" => self.open_collection("liked"),
            _ => self.play_context(id),
        }
    }

    /// Botão de tocar de um destaque: toca na hora, sem abrir a página.
    pub fn play_item(&mut self, kind: &str, id: &str) {
        let what = match (kind, ytm_vm::strip_remote(id)) {
            ("album" | "playlist", Some(raw)) => Some(RemoteRef::album_or_playlist(raw)),
            ("artist", Some(raw)) => Some(RemoteRef::Artist(raw.to_owned())),
            ("mood", _) => id.strip_prefix("mood:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| self.categories.get(i)).map(|c| RemoteRef::Browse { title: c.title.clone(), target: c.browse.clone() }),
            _ => None,
        };
        match what {
            Some(what) => {
                // invalida qualquer pedido de tocar anterior ainda em voo
                self.ctx_seq += 1;
                self.engine_radio = None;
                self.toast("Procurando as músicas…", false);
                self.jobs.submit(Job::Remote(RemoteJob::PlayItem { ctx_seq: self.ctx_seq, what, shuffle: false }));
            }
            None => self.open_item(kind, id),
        }
    }

    fn open_remote(&mut self, what: RemoteRef) {
        self.visit(super::DetailRoute::Catalog(what));
    }

    pub(super) fn load_remote_page(&mut self, what: RemoteRef) {
        self.detail_seq += 1;
        self.detail_what = Some(what.clone());
        self.detail_album = None;
        self.detail_ids.clear();
        self.show_detail(DetailVm { state: LoadStateVm::Loading, title: self.detail_vm.title.clone(), ..Default::default() });
        self.jobs.submit(Job::Remote(RemoteJob::Open { seq: self.detail_seq, what }));
    }

    pub fn retry_detail(&mut self) {
        self.restore_visit();
    }

    pub fn toggle_detail_saved(&mut self) {
        let Some(album) = self.detail_album.clone() else { return };
        let saved = !self.detail_vm.saved;
        self.detail_vm.saved = saved;
        let vm = self.detail_vm.clone();
        self.view.set_detail(&vm);
        self.jobs.submit(Job::Remote(RemoteJob::SaveAlbum { album, saved }));
        self.toast(if saved { "Álbum guardado na biblioteca. Fica só neste aparelho" } else { "Álbum removido da biblioteca" }, false);
    }

    pub fn goto_album(&mut self, track_id: &str) {
        if ytm_vm::is_remote_id(track_id) {
            match self.remote_tracks.peek(&track_id.to_owned()).and_then(|t| t.album.as_ref()).and_then(|a| a.id.as_ref()).map(|id| id.as_str().to_owned()) {
                Some(id) => self.open_remote(RemoteRef::Album(id)),
                None => self.toast("Esta música não tem página de álbum.", false),
            }
        } else {
            self.request_detail(DetailRef::AlbumOfTrack(track_id.to_owned()));
        }
    }

    pub fn goto_artist(&mut self, track_id: &str) {
        if ytm_vm::is_remote_id(track_id) {
            match self.remote_tracks.peek(&track_id.to_owned()).and_then(|t| t.artists.first()).and_then(|a| a.id.as_ref()).map(|id| id.as_str().to_owned()) {
                Some(id) => self.open_remote(RemoteRef::Artist(id)),
                None => self.toast("Não achei a página deste artista.", false),
            }
        } else {
            self.request_detail(DetailRef::ArtistOfTrack(track_id.to_owned()));
        }
    }

    // ----------------------------------------------------------------------------------------
    // Busca
    // ----------------------------------------------------------------------------------------

    /// A cada edição do campo: o worker faz o debounce (≈250 ms) e descarta o que ficou velho.
    pub fn search(&mut self, text: &str) {
        self.search_query = text.trim().to_owned();
        self.search_seq += 1;
        self.search_remote = None;
        self.search_remote_failed = None;
        self.search_local_songs.clear();
        self.search_local_albums.clear();
        self.search_local_ids.clear();
        self.search_suggestions.clear();
        self.search_local_pending = false;
        if self.search_query.is_empty() {
            self.publish_search();
            return;
        }
        if self.search_source != 1 {
            self.search_local_pending = true;
            self.jobs.submit(Job::Search { seq: self.search_seq, query: self.search_query.clone() });
        }
        if self.remote_enabled && self.search_source != 2 {
            self.jobs.submit(Job::Remote(RemoteJob::Search { seq: self.search_seq, query: self.search_query.clone(), kind: self.search_kind }));
            self.jobs.submit(Job::Remote(RemoteJob::Suggest { seq: self.search_seq, prefix: self.search_query.clone() }));
        }
        self.publish_search();
    }

    /// 0 tudo, 1 músicas, 2 álbuns, 3 artistas, 4 playlists.
    pub fn search_kind_changed(&mut self, kind: i32) {
        self.search_kind = kind;
        let q = self.search_query.clone();
        self.search(&q);
    }

    pub fn search_source_changed(&mut self, source: i32) {
        self.search_source = source.clamp(0, 2);
        self.search(&self.search_query.clone());
    }

    pub(super) fn publish_search(&mut self) {
        let mut vm = if self.search_query.is_empty() {
            SearchVm::default()
        } else {
            let kind = self.search_kind;
            let mut vm = self.search_remote.clone().unwrap_or_default();
            if kind <= 1 {
                vm.songs.extend(self.search_local_songs.iter().cloned());
            }
            if kind == 0 || kind == 2 {
                vm.albums.extend(self.search_local_albums.iter().cloned());
            }
            let any = !(vm.songs.is_empty() && vm.albums.is_empty() && vm.artists.is_empty() && vm.playlists.is_empty());
            let pending = self.search_local_pending || (self.remote_enabled && self.search_source != 2 && self.search_remote.is_none() && self.search_remote_failed.is_none());
            vm.state = if any {
                SearchStateVm::Results
            } else if pending {
                SearchStateVm::Loading
            } else {
                match self.search_remote_failed {
                    Some(k) if ytm_vm::is_offline(k) => SearchStateVm::Offline,
                    Some(k) => {
                        vm.error_text = ytm_vm::error_copy(k).1.into();
                        SearchStateVm::Error
                    }
                    None => SearchStateVm::Empty,
                }
            };
            if vm.state == SearchStateVm::Offline {
                vm.error_text = "Sem conexão. A busca só olha o que está no aparelho.".into();
            }
            vm.empty_text = format!("Nada para “{}”. Confira a grafia ou busque só o nome do artista.", self.search_query);
            vm.suggestions = self.search_suggestions.clone();
            vm
        };
        vm.recent = self.cfg.recent_searches.clone();
        self.flag_tracks(&mut vm.songs);
        self.cover_pass(&mut vm);
        self.view.set_search(&vm);
    }

    // ----------------------------------------------------------------------------------------
    // Tocar: contextos
    // ----------------------------------------------------------------------------------------

    fn submit_local_play(&mut self, scope: Scope, start_id: Option<String>, shuffle: bool, label: String) {
        if self.engine_quality.is_some() || cfg!(any(target_os = "android", target_os = "ios")) {
            self.ctx_seq += 1;
            self.ctx = None;
            self.engine_radio = None;
            self.jobs.submit(Job::PlayScoped { seq: self.ctx_seq, scope, start_id, shuffle, label });
        } else {
            self.jobs.submit(Job::Play { scope, start_id, shuffle, label });
        }
    }

    /// Id da faixa → lista de origem → fila. Listas só locais seguem para o motor inteiras; qualquer lista com
    /// faixa remota vira um contexto sob demanda.
    pub(super) fn play_context(&mut self, id: &str) {
        if let Some(track_id) = id.strip_prefix("sp:") {
            if !crate::platform::open_spotify_track(track_id) {
                self.toast("Não foi possível abrir a faixa no Spotify.", true);
            }
            return;
        }
        // ids especiais dos cartões locais
        if let Some(album) = id.strip_prefix("album:") {
            self.submit_local_play(Scope::Album(album.to_owned()), None, false, String::new());
            return;
        }
        if id == "liked" {
            self.submit_local_play(Scope::Liked, None, false, "Curtidos".into());
            return;
        }
        if id == "shuffle-all" {
            self.submit_local_play(Scope::All, None, true, "Biblioteca".into());
            return;
        }
        // de onde veio o clique
        let search_ids: Vec<String> = if self.tab == NavTab::Search { self.search_play_ids() } else { vec![] };
        let in_home_picks = self.home.quick_picks.iter().any(|t| t.id == id);
        let in_trending = self.explore.trending.iter().any(|t| t.id == id);
        let (list, label, whole) = if self.detail_open && self.detail_ids.iter().any(|t| t == id) {
            (self.detail_ids.clone(), self.detail_title.clone(), true)
        } else if search_ids.iter().any(|t| t == id) {
            self.remember_search();
            if ytm_vm::is_remote_id(id) {
                (vec![id.to_owned()], "Busca".to_owned(), false)
            } else {
                (self.search_local_ids.clone(), "Busca".to_owned(), true)
            }
        } else if self.recents_ids.iter().any(|t| t == id) {
            (self.recents_ids.clone(), "Recentes".to_owned(), true)
        } else if in_home_picks {
            (vec![id.to_owned()], "Início".to_owned(), false)
        } else if in_trending {
            (vec![id.to_owned()], "Em alta".to_owned(), false)
        } else {
            (vec![id.to_owned()], if ytm_vm::is_remote_id(id) { ytm_vm::SOURCE_NAME.to_owned() } else { "Biblioteca".to_owned() }, false)
        };
        let any_remote = list.iter().any(|i| ytm_vm::is_remote_id(i));
        if !any_remote {
            let start = Some(id.to_owned());
            self.submit_local_play(Scope::Ids(if whole { list } else { vec![id.to_owned()] }), start, false, label);
            return;
        }
        let list = if whole { list } else { vec![id.to_owned()] };
        let entries = self.entries_for(&list);
        let Some(start) = entries.iter().position(|e| e.id() == id) else {
            self.toast("Não consegui abrir esta música. Atualize a tela e tente de novo.", true);
            return;
        };
        self.start_context(entries, start, &label, false);
    }

    /// Ids tocáveis da busca, na ordem mostrada: remotos primeiro, depois os locais.
    fn search_play_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.search_remote.iter().flat_map(|r| r.songs.iter().map(|t| t.id.clone())).collect();
        ids.extend(self.search_local_ids.iter().cloned());
        ids
    }

    fn entries_for(&mut self, ids: &[String]) -> Vec<Entry> {
        ids.iter()
            .filter_map(|id| {
                if ytm_vm::is_remote_id(id) {
                    self.remote_tracks.get(id).cloned().map(|t| Entry::Remote(Box::new(t)))
                } else {
                    Some(Entry::Local(id.clone()))
                }
            })
            .collect()
    }

    /// Toca o detalhe aberto (ou o embaralha).
    pub fn play_all(&mut self, shuffle: bool) {
        if self.detail_ids.is_empty() {
            return;
        }
        let ids = self.detail_ids.clone();
        if ids.iter().any(|i| ytm_vm::is_remote_id(i)) {
            let entries = self.entries_for(&ids);
            if !entries.is_empty() {
                let title = self.detail_title.clone();
                self.start_context(entries, 0, &title, shuffle);
            }
        } else {
            self.submit_local_play(Scope::Ids(ids), None, shuffle, self.detail_title.clone());
        }
    }

    pub(super) fn start_context(&mut self, mut entries: Vec<Entry>, start: usize, label: &str, shuffle_all: bool) {
        if entries.is_empty() {
            return;
        }
        if self.engine_quality.is_some() {
            self.ctx = None;
            self.engine_radio = None;
            self.ctx_seq += 1;
            self.jobs.submit(Job::PrepareQueue {
                seq: self.ctx_seq, entries, start, shuffle: shuffle_all, label: label.to_owned(),
            });
            return;
        }
        let start = start.min(entries.len() - 1);
        let start_entry = if shuffle_all {
            shuffle(&mut entries);
            0
        } else {
            start
        };
        self.ctx_seq += 1;
        let first = entries[start_entry].clone();
        let tail: VecDeque<Entry> = entries[start_entry + 1..].iter().cloned().collect();
        let radio_seed = entries.iter().rev().find_map(|e| match e {
            Entry::Remote(t) => Some(t.id.as_str().to_owned()),
            Entry::Local(_) => None,
        });
        let repeat_all = self.pb.repeat == RepeatVm::All;
        self.ctx = Some(Context {
            seq: self.ctx_seq,
            tail,
            all: entries,
            shuffled: shuffle_all,
            repeat_all,
            start_pending: true,
            start: first.clone(),
            resolving: false,
            stalled: false,
            radio_seed,
            radio: None,
            radio_fetching: false,
            radio_exhausted: false,
            radio_empty_rounds: 0,
            resume: false,
            fail_count: 0,
            retry_after: None,
            played: HashSet::new(),
        });
        // o embaralhar do motor não vale aqui: a ordem é do contexto
        if self.pb.shuffle && !shuffle_all {
            self.engine.set_shuffle(false);
        } else if shuffle_all {
            self.engine.set_shuffle(false);
            self.pb.shuffle = true;
        }
        self.stalled = false;
        self.pb.origin = label.to_owned();
        self.pb.error.clear();
        self.pb.loading = true;
        self.view.set_playback(&self.pb);
        self.submit_resolve(&first, true);
        self.publish_queue();
    }

    pub(super) fn install_engine_radio(&mut self, seq: u64, items: &[QueueItem], start: usize) {
        let Some(first) = items.get(start).map(|item| item.id) else { return };
        self.engine_radio = items.iter().rev().find(|item| matches!(&item.track.source, TrackSource::Remote { .. }))
            .map(|item| EngineRadio {
                seq, first, seed: item.track.id.to_string(), continuation: None,
                fetching: false, exhausted: false, empty_rounds: 0, stopped: false, retry_after: None,
            });
    }

    pub(super) fn cancel_radio_resume(&mut self) {
        if let Some(state) = &mut self.engine_radio {
            state.stopped = false;
        }
        if let Some(state) = &mut self.ctx {
            state.resume = false;
        }
    }

    pub(super) fn retry_engine_radio(&mut self, seq: u64) {
        if self.engine_radio.as_ref().is_some_and(|state| state.seq == seq && state.stopped) {
            self.request_engine_radio();
        }
    }

    fn request_engine_radio(&mut self) {
        if !self.cfg.autoplay_radio || self.pb.repeat != RepeatVm::Off || self.upcoming().len() > LOOKAHEAD {
            return;
        }
        let Some(radio) = self.engine_radio.as_mut() else { return };
        // QueueChanged may still refer to the previous load.
        if !self.queue_items.iter().any(|item| item.id == radio.first)
            || radio.fetching || radio.exhausted
            || radio.retry_after.is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        radio.fetching = true;
        self.jobs.submit(Job::Remote(RemoteJob::Radio {
            ctx_seq: radio.seq, seed: radio.seed.clone(), prior: radio.continuation.clone(),
        }));
    }

    fn append_engine_radio(&mut self, seq: u64, tracks: Vec<CoreTrack>, radio: Radio) -> bool {
        let Some(state) = self.engine_radio.as_mut().filter(|state| state.seq == seq) else { return false };
        state.fetching = false;
        if !self.cfg.autoplay_radio {
            return true;
        }
        let mut known: HashSet<String> = self.queue_items.iter().filter_map(|item| {
            matches!(&item.track.source, TrackSource::Remote { .. }).then(|| item.track.id.to_string())
        }).collect();
        let items: Vec<_> = tracks.into_iter().filter(|track| known.insert(track.id.to_string()))
            .take(RADIO_BATCH).map(|track| QueueItem::new(track, kazzu_core::StreamSource::Deferred)).collect();
        let has_more = radio.continuation.is_some();
        state.continuation = Some(Box::new(radio));
        if items.is_empty() {
            state.empty_rounds += 1;
            state.exhausted = !has_more || state.empty_rounds >= 3;
            self.request_engine_radio();
        } else {
            state.empty_rounds = 0;
            state.exhausted = !has_more;
            let first = items[0].id;
            let resume = std::mem::take(&mut state.stopped);
            for item in &items {
                self.remember_remote(&item.track);
            }
            self.engine.enqueue(items);
            if resume {
                self.engine.jump_to(first);
            }
        }
        true
    }

    fn submit_resolve(&self, entry: &Entry, for_start: bool) {
        let ctx_seq = self.ctx_seq;
        match entry {
            Entry::Local(id) => self.jobs.submit(Job::ResolveLocal { ctx_seq, id: id.clone(), for_start }),
            Entry::Remote(t) => self.jobs.submit(Job::Remote(RemoteJob::Resolve { ctx_seq, track: (**t).clone(), quality: self.quality(), for_start })),
        }
    }

    /// Marca a fila sob demanda como parada por falha de rede: espera com recuo exponencial (2 s, 4 s … 30 s) antes de
    /// tentar a próxima de novo, então nunca martela o servidor e se recupera sozinha.
    fn stall(&mut self) {
        if let Some(c) = self.ctx.as_mut() {
            c.fail_count = c.fail_count.saturating_add(1);
            let secs = (1u64 << c.fail_count.min(5)).min(30);
            c.retry_after = Some(Instant::now() + Duration::from_secs(secs));
            c.stalled = true;
        }
    }

    /// Sai do estado parado e tenta a próxima. Sem `force`, só depois do recuo.
    pub(super) fn unstall(&mut self, force: bool) {
        if self.engine_radio.is_some() {
            self.request_engine_radio();
            return;
        }
        let due = match self.ctx.as_ref() {
            Some(c) if c.stalled => force || c.retry_after.is_none_or(|t| Instant::now() >= t),
            _ => false,
        };
        if due {
            if let Some(c) = self.ctx.as_mut() {
                c.stalled = false;
            }
            self.ensure_lookahead();
        }
    }

    /// A lista acabou de verdade (nada a resolver, sem rádio): o "retomar" pendente não tem mais o que retomar.
    fn check_end(&mut self) {
        let autoplay = self.cfg.autoplay_radio && self.remote_enabled;
        let ended = self.ctx.as_ref().is_some_and(|c| c.resume && c.tail.is_empty() && !c.resolving && !c.start_pending && !c.radio_fetching && !c.stalled && (c.radio_exhausted || !autoplay || c.radio_seed.is_none()));
        if ended {
            if let Some(c) = self.ctx.as_mut() {
                c.resume = false;
            }
            self.pb.loading = false;
            self.view.set_playback(&self.pb);
        }
    }

    /// O motor chegou ao fim da fila. Numa fila sob demanda isso pode ser só "a próxima ainda não chegou" (rede lenta,
    /// Próxima várias vezes): fica marcado para retomar assim que a faixa for entregue.
    pub(super) fn on_queue_finished(&mut self) {
        if let Some(radio) = self.engine_radio.as_mut() {
            radio.stopped = self.cfg.autoplay_radio;
            self.request_engine_radio();
            return;
        }
        let autoplay = self.cfg.autoplay_radio && self.remote_enabled;
        let Some(c) = self.ctx.as_mut() else { return };
        let more = c.resolving || c.start_pending || !c.tail.is_empty() || c.radio_fetching || c.repeat_all || (autoplay && !c.radio_exhausted && c.radio_seed.is_some());
        c.resume = more;
        if more {
            self.pb.loading = true;
            self.view.set_playback(&self.pb);
            self.ensure_lookahead();
            self.check_end();
        }
    }

    fn on_resolved(&mut self, ctx_seq: u64, item: QueueItem, quality_label: String, for_start: bool) {
        if self.ctx.as_ref().map(|c| c.seq) != Some(ctx_seq) {
            return;
        }
        if matches!(item.track.source, TrackSource::Remote { .. }) {
            self.set_online(true);
            self.remember_remote(&item.track);
        }
        if !quality_label.is_empty() {
            self.quality_labels.insert(item.id, quality_label);
        }
        if for_start {
            if let Some(c) = self.ctx.as_mut() {
                c.start_pending = false;
                c.resume = false;
                c.fail_count = 0;
            }
            self.engine.load(vec![item], 0);
        } else {
            let id = item.id;
            let resume = self.ctx.as_mut().is_some_and(|c| {
                c.resolving = false;
                c.fail_count = 0;
                std::mem::take(&mut c.resume)
            });
            self.engine.enqueue(vec![item]);
            if resume {
                // o motor tinha parado no fim da fila esperando esta faixa: toca-a agora
                self.engine.jump_to(id);
            }
        }
    }

    fn on_resolve_failed(&mut self, ctx_seq: u64, track: CoreTrack, kind: ErrorKind, for_start: bool) {
        if self.ctx.as_ref().map(|c| c.seq) != Some(ctx_seq) {
            return;
        }
        if ytm_vm::is_offline(kind) {
            self.set_online(false);
        }
        let (title, text, retry) = ytm_vm::error_copy(kind);
        let msg = format!("{title}. {text}");
        let failed_id = if matches!(track.source, TrackSource::Remote { .. }) { ytm_vm::remote_id(track.id.as_str()) } else { track.id.to_string() };
        if !retry && let Some(c) = self.ctx.as_mut() {
            // indisponível de vez (região, removida): sai da lista, senão "repetir tudo" daria voltas nela
            c.all.retain(|e| e.id() != failed_id);
        }
        if for_start {
            if retry {
                self.pb.loading = false;
                self.view.set_playback(&self.pb);
                self.toast_with(&msg, true, "Tentar de novo", ToastAction::RetryStart);
            } else {
                // não adianta tentar de novo: pula para a próxima da lista, ainda "carregando"
                self.toast(&msg, true);
                let next = self.ctx.as_mut().and_then(|c| c.tail.pop_front());
                match next {
                    Some(next) => {
                        if let Some(c) = self.ctx.as_mut() {
                            c.start = next.clone();
                        }
                        self.submit_resolve(&next, true);
                    }
                    None => {
                        if let Some(c) = self.ctx.as_mut() {
                            c.start_pending = false;
                        }
                        self.pb.loading = false;
                        self.view.set_playback(&self.pb);
                    }
                }
            }
            return;
        }
        if let Some(c) = self.ctx.as_mut() {
            c.resolving = false;
        }
        if retry {
            // rede: devolve a faixa ao início da fila e espera (recuo), sem martelar o servidor
            if let Some(c) = self.ctx.as_mut() {
                c.tail.push_front(Entry::Remote(Box::new(track)));
            }
            self.stall();
            self.toast_with("Sem conexão. As próximas músicas esperam; o Kazzu tenta de novo sozinho.", true, "Tentar de novo", ToastAction::RetryLookahead);
            self.publish_queue();
        } else {
            self.toast(&format!("Pulei “{}”. {text}", track.title), false);
            self.ensure_lookahead();
            self.check_end();
        }
    }

    pub(super) fn retry_start(&mut self) {
        let Some(c) = self.ctx.as_mut() else { return };
        c.start_pending = true;
        let start = c.start.clone();
        self.pb.loading = true;
        self.view.set_playback(&self.pb);
        self.submit_resolve(&start, true);
    }

    /// Mantém `LOOKAHEAD` faixas resolvidas à frente da atual; acabando a lista, pede o rádio.
    pub(super) fn ensure_lookahead(&mut self) {
        if self.engine_radio.is_some() {
            self.request_engine_radio();
            return;
        }
        enum Next {
            Nothing,
            Resolve(Entry),
            Radio { seq: u64, seed: String, prior: Option<Box<Radio>> },
        }
        let ready = self.upcoming().len();
        let autoplay = self.cfg.autoplay_radio && self.remote_enabled;
        let repeat_all = self.pb.repeat == RepeatVm::All;
        let next = {
            let Some(ctx) = self.ctx.as_mut() else { return };
            if ctx.start_pending || ctx.resolving || ctx.stalled || ready >= LOOKAHEAD {
                return;
            }
            if ctx.tail.is_empty() && repeat_all && !ctx.all.is_empty() {
                // repetir tudo: a lista original volta (já sem as indisponíveis)
                ctx.tail = ctx.all.iter().cloned().collect();
            }
            if let Some(entry) = ctx.tail.pop_front() {
                ctx.resolving = true;
                Next::Resolve(entry)
            } else if autoplay && !ctx.radio_exhausted && !ctx.radio_fetching && ctx.radio_seed.is_some() {
                ctx.radio_fetching = true;
                Next::Radio { seq: ctx.seq, seed: ctx.radio_seed.clone().unwrap_or_default(), prior: ctx.radio.clone() }
            } else {
                Next::Nothing
            }
        };
        match next {
            Next::Resolve(entry) => {
                self.submit_resolve(&entry, false);
                self.publish_queue();
            }
            Next::Radio { seq, seed, prior } => self.jobs.submit(Job::Remote(RemoteJob::Radio { ctx_seq: seq, seed, prior })),
            Next::Nothing => {}
        }
    }

    fn on_radio(&mut self, ctx_seq: u64, tracks: Vec<CoreTrack>, radio: Radio) {
        if self.engine_radio.as_ref().is_some_and(|state| state.seq == ctx_seq) {
            self.append_engine_radio(ctx_seq, tracks, radio);
            return;
        }
        if self.ctx.as_ref().map(|c| c.seq) != Some(ctx_seq) {
            return;
        }
        self.set_online(true);
        let more = radio.continuation.is_some();
        let mut fresh: Vec<CoreTrack> = Vec::new();
        if let Some(ctx) = self.ctx.as_mut() {
            ctx.radio_fetching = false;
            ctx.fail_count = 0;
            let known: HashSet<String> = ctx.all.iter().map(Entry::id).chain(ctx.played.iter().cloned()).chain(ctx.tail.iter().map(Entry::id)).collect();
            fresh = tracks.into_iter().filter(|t| !known.contains(&ytm_vm::remote_id(t.id.as_str()))).take(RADIO_BATCH).collect();
            ctx.radio = Some(Box::new(radio));
            if fresh.is_empty() {
                ctx.radio_empty_rounds += 1;
                if !more || ctx.radio_empty_rounds >= 3 {
                    ctx.radio_exhausted = true;
                }
            } else {
                ctx.radio_empty_rounds = 0;
                ctx.tail.extend(fresh.iter().cloned().map(|t| Entry::Remote(Box::new(t))));
            }
        }
        for t in &fresh {
            self.remember_remote(t);
        }
        self.publish_queue();
        self.ensure_lookahead();
        self.check_end();
    }

    /// O rádio falhou: erro de rede NÃO esgota o rádio (recua e tenta de novo); só o que não tem conserto esgota.
    pub(super) fn on_radio_failed(&mut self, ctx_seq: u64, kind: ErrorKind) {
        if let Some(state) = self.engine_radio.as_mut().filter(|state| state.seq == ctx_seq) {
            state.fetching = false;
            state.exhausted = !kind.is_retryable();
            let delay = Duration::from_secs(30);
            state.retry_after = Some(Instant::now() + delay);
            if !state.exhausted && state.stopped {
                let sink = self.sink.clone();
                let _ = std::thread::Builder::new().name("kazzu-radio-retry".into()).spawn(move || {
                    std::thread::sleep(delay);
                    sink(Msg::RetryEngineRadio(ctx_seq));
                });
            }
            return;
        }
        if self.ctx.as_ref().map(|c| c.seq) != Some(ctx_seq) {
            return;
        }
        if let Some(c) = self.ctx.as_mut() {
            c.radio_fetching = false;
        }
        if kind.is_retryable() || ytm_vm::is_offline(kind) {
            if ytm_vm::is_offline(kind) {
                self.set_online(false);
            }
            self.stall();
        } else {
            if let Some(c) = self.ctx.as_mut() {
                c.radio_exhausted = true;
            }
            self.check_end();
        }
    }

    /// Toca uma linha que ainda não entrou no motor: descarta as anteriores e começa por ela. Incrementa a geração do
    /// contexto, então qualquer resolução em andamento (da faixa anterior ou de uma "próxima" já pulada) é descartada.
    pub(super) fn jump_in_tail(&mut self, j: usize) {
        let Some(ctx) = self.ctx.as_mut() else { return };
        if j >= ctx.tail.len() {
            return;
        }
        ctx.tail.drain(..j);
        let Some(entry) = ctx.tail.pop_front() else { return };
        self.ctx_seq += 1;
        ctx.seq = self.ctx_seq;
        ctx.start = entry.clone();
        ctx.start_pending = true;
        ctx.resolving = false;
        ctx.radio_fetching = false;
        ctx.stalled = false;
        ctx.resume = false;
        self.pb.loading = true;
        self.view.set_playback(&self.pb);
        self.submit_resolve(&entry, true);
    }

    /// Entrada do contexto para uma faixa que já está no motor (para devolvê-la à fila sob demanda).
    fn entry_of(item: &QueueItem) -> Entry {
        match &item.track.source {
            TrackSource::Remote { .. } => Entry::Remote(Box::new(item.track.clone())),
            TrackSource::Local { .. } => Entry::Local(item.track.id.to_string()),
        }
    }

    /// Id da linha (como a UI o conhece) de uma faixa do motor.
    fn row_id(item: &QueueItem) -> String {
        match &item.track.source {
            TrackSource::Remote { .. } => ytm_vm::remote_id(item.track.id.as_str()),
            TrackSource::Local { .. } => item.track.id.to_string(),
        }
    }

    /// Onde está a linha `id`: índice entre as próximas do motor, ou na parte ainda não entregue (`tail`).
    fn locate_row(&self, id: &str) -> Option<RowPos> {
        if let Some(i) = self.upcoming().iter().position(|it| Self::row_id(it) == id) {
            return Some(RowPos::Engine(i));
        }
        self.ctx.as_ref().and_then(|c| c.tail.iter().position(|e| e.id() == id)).map(RowPos::Tail)
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_queue_action(&mut self, action: &str, id: &str) {
        let Ok(id) = id.parse::<u64>() else { return };
        let id = QueueItemId(id);
        let has_focus = self.android_has_focus();
        let upcoming = self.upcoming();
        let Some(index) = upcoming.iter().position(|item| item.id == id) else { return };
        match action {
            "jump" if has_focus => self.engine.jump_to(id),
            "remove" => self.engine.remove(id),
            "move" if index > 0 => self.engine.move_before(id, upcoming[index - 1].id),
            _ => {}
        }
    }

    pub fn queue_remove(&mut self, id: &str) {
        match self.locate_row(id) {
            Some(RowPos::Engine(i)) => {
                if let Some(item) = self.upcoming().get(i) {
                    self.engine.remove(item.id);
                }
            }
            Some(RowPos::Tail(j)) => {
                if let Some(c) = self.ctx.as_mut() {
                    c.tail.remove(j);
                }
                self.publish_queue();
            }
            None => {}
        }
    }

    pub fn queue_jump(&mut self, id: &str) {
        match self.locate_row(id) {
            Some(RowPos::Engine(i)) => {
                if let Some(item) = self.upcoming().get(i) {
                    self.engine.jump_to(item.id);
                }
            }
            Some(RowPos::Tail(j)) => self.jump_in_tail(j),
            None => {}
        }
    }

    /// Seta "Mover para cima": troca a linha com a de cima (a primeira de "A seguir" não tem seta).
    pub fn queue_move_up(&mut self, id: &str) {
        match self.locate_row(id) {
            Some(RowPos::Engine(i)) if i > 0 => {
                let upcoming = self.upcoming();
                self.engine.move_before(upcoming[i].id, upcoming[i - 1].id);
            }
            Some(RowPos::Tail(j)) if j > 0 => {
                if let Some(c) = self.ctx.as_mut() {
                    c.tail.swap(j - 1, j);
                }
                self.publish_queue();
            }
            Some(RowPos::Tail(0)) => {
                // a primeira que falta entrar passa à frente da última que o motor já tem
                let last = self.upcoming().last().cloned();
                let busy = self.ctx.as_ref().is_none_or(|c| c.resolving || c.start_pending);
                if let (Some(last), false) = (last, busy) {
                    let moved = self.ctx.as_mut().and_then(|c| {
                        let m = c.tail.pop_front();
                        c.tail.push_front(Self::entry_of(&last));
                        c.resolving = m.is_some();
                        m
                    });
                    self.engine.remove(last.id);
                    if let Some(m) = moved {
                        self.submit_resolve(&m, false);
                    }
                    self.publish_queue();
                }
            }
            _ => {}
        }
    }

    pub fn toggle_shuffle(&mut self) {
        let Some(ctx) = self.ctx.as_mut() else {
            self.engine.set_shuffle(!self.pb.shuffle);
            return;
        };
        if ctx.shuffled {
            // volta à ordem original, só do que ainda não foi tocado
            let pending: HashSet<String> = ctx.tail.iter().map(Entry::id).collect();
            ctx.tail = ctx.all.iter().filter(|e| pending.contains(&e.id())).cloned().collect();
            ctx.shuffled = false;
        } else {
            let mut v: Vec<Entry> = ctx.tail.drain(..).collect();
            shuffle(&mut v);
            ctx.tail = v.into();
            ctx.shuffled = true;
        }
        self.pb.shuffle = ctx.shuffled;
        self.view.set_playback(&self.pb);
        self.publish_queue();
    }

    pub fn cycle_repeat(&mut self) {
        let next = match self.pb.repeat {
            RepeatVm::Off => RepeatVm::All,
            RepeatVm::All => RepeatVm::One,
            RepeatVm::One => RepeatVm::Off,
        };
        if let Some(ctx) = self.ctx.as_mut() {
            // numa fila sob demanda, "repetir tudo" é do contexto; o motor só repete uma faixa
            ctx.repeat_all = next == RepeatVm::All;
            self.pb.repeat = next;
            self.engine.set_repeat(if next == RepeatVm::One { RepeatMode::One } else { RepeatMode::Off });
            self.view.set_playback(&self.pb);
            self.ensure_lookahead();
        } else {
            self.engine.set_repeat(match next {
                RepeatVm::Off => RepeatMode::Off,
                RepeatVm::All => RepeatMode::All,
                RepeatVm::One => RepeatMode::One,
            });
        }
    }

    // ----------------------------------------------------------------------------------------
    // Streaming: URL vencida e rede
    // ----------------------------------------------------------------------------------------

    pub(super) fn on_stream(&mut self, ev: StreamEvent) {
        match ev {
            StreamEvent::Expired { id, .. } => {
                // a URL assinada venceu (ou a rede mudou): resolve de novo e entrega ao motor, que retoma na posição.
                // No máximo 2 tentativas em 60 s por faixa: se a URL nova também for recusada, é a ação do usuário.
                let now = Instant::now();
                let log = self.expired_log.entry(id).or_default();
                log.retain(|t| now.duration_since(*t) < EXPIRED_WINDOW);
                if log.len() >= EXPIRED_MAX {
                    self.pb.error = "Não deu para renovar a música. Toque para tentar de novo.".into();
                    self.pb.error_action = "Tentar de novo".into();
                    self.view.set_playback(&self.pb);
                    self.toast_with("O YouTube Music recusou a música de novo", true, "Tentar de novo", ToastAction::RetryReresolve(id));
                    return;
                }
                log.push(now);
                self.reresolve(id);
            }
            StreamEvent::NetworkFailed { message, .. } => {
                log::warn!("rede: {message}");
                self.set_online(false);
                self.stalled = true;
                self.pb.error = "Sem conexão. Toque para tentar de novo.".into();
                self.pb.error_action = "Tentar de novo".into();
                self.view.set_playback(&self.pb);
                self.toast_with("Sem conexão, toque para tentar", true, "Tentar de novo", ToastAction::RetryPlay);
            }
        }
    }

    /// Resolve de novo a faixa `id` do motor (URL vencida).
    pub(super) fn reresolve(&mut self, id: QueueItemId) {
        let track = self.queue_items.iter().find(|i| i.id == id).map(|i| i.track.clone());
        match track {
            Some(track) if matches!(track.source, TrackSource::Remote { .. }) => {
                self.jobs.submit(Job::Remote(RemoteJob::Reresolve { id, track, quality: self.quality() }));
            }
            _ => log::warn!("URL vencida de uma faixa que não é remota: {id:?}"),
        }
    }
}
