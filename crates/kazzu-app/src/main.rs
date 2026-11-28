//! Binário `kazzu`: app desktop.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop {

use std::path::PathBuf;
use std::process::ExitCode;

use kazzu_app::app::{App, AppError, Options};
use slint::ComponentHandle;

const HELP: &str = "\
Kazzu, player de música

Uso: kazzu [opções]

  --demo-library <dir>   usa uma biblioteca de demonstração gerada em <dir> (não toca na sua)
  --data-dir <dir>       onde ficam o banco e o cache de capas
  --config <arquivo>     arquivo de configuração (TOML)
  --add-folder <dir>     indexa também esta pasta de músicas (pode repetir)
  --no-youtube           só arquivos locais: não usa o YouTube Music (nenhuma chamada de rede)
  --null-audio           saída de áudio simulada (sem placa de som)
  --renderer <nome>      femtovg (padrão, com vidro desfocado) ou software (sem blur)
  --snapshot <dir>       gera capturas de tela de todas as telas em <dir> e sai
  --size <LxA>           tamanho inicial da janela, ex.: 1280x800
  -h, --help             esta ajuda
";

#[derive(Default)]
struct Cli {
    opts: Options,
    demo: Option<PathBuf>,
    renderer: Option<String>,
    snapshot: Option<PathBuf>,
    size: Option<(f32, f32)>,
    help: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} precisa de um valor"));
        match a.as_str() {
            "--demo-library" => cli.demo = Some(PathBuf::from(value("--demo-library")?)),
            "--data-dir" => cli.opts.data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--config" => cli.opts.config_path = Some(PathBuf::from(value("--config")?)),
            "--add-folder" => cli.opts.extra_folders.push(PathBuf::from(value("--add-folder")?)),
            "--null-audio" => cli.opts.null_audio = true,
            "--no-youtube" => cli.opts.no_remote = true,
            "--renderer" => cli.renderer = Some(value("--renderer")?),
            "--snapshot" => cli.snapshot = Some(PathBuf::from(value("--snapshot")?)),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("use --size LARGURAxALTURA")?;
                cli.size = Some((w.parse().map_err(|_| "largura inválida")?, h.parse().map_err(|_| "altura inválida")?));
            }
            "-h" | "--help" => cli.help = true,
            other => return Err(format!("opção desconhecida: {other}")),
        }
    }
    Ok(cli)
}

/// Escolhe o backend/renderer. Devolve `true` se o renderer é o de software (sem `backdrop-blur`).
fn select_renderer(wanted: Option<&str>) -> bool {
    let name = wanted.unwrap_or("femtovg");
    let selected = slint::BackendSelector::new().backend_name("winit".into()).renderer_name(name.into()).select();
    match selected {
        Ok(()) => name == "software",
        Err(e) => {
            eprintln!("renderer {name} indisponível ({e}); usando o software, sem vidro desfocado");
            let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select();
            true
        }
    }
}

fn run() -> Result<(), AppError> {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    if cli.help {
        print!("{HELP}");
        return Ok(());
    }
    let mut opts = cli.opts;
    #[cfg(feature = "demo")]
    if let Some(dir) = &cli.demo {
        let music = kazzu_app::demo::ensure_demo_music(dir).map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        opts.data_dir.get_or_insert_with(|| dir.join("data"));
        opts.config_path.get_or_insert_with(|| dir.join("config.toml"));
        opts.extra_folders.push(music);
    }
    #[cfg(not(feature = "demo"))]
    if cli.demo.is_some() {
        eprintln!("esta compilação não inclui a biblioteca de demonstração (feature `demo`)");
        std::process::exit(2);
    }
    opts.software_renderer = select_renderer(cli.renderer.as_deref());
    // capturas sempre partem de uma configuração limpa (e não sobrescrevem a do usuário nem a da demonstração)
    if let Some(dir) = &cli.snapshot {
        let cfg = dir.join(".snapshot-config.toml");
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::remove_file(&cfg);
        opts.config_path = Some(cfg);
    }

    // renderer padrão (FemtoVG): `App::build` o testa antes de subir motor, worker e banco
    opts.probe_renderer = !opts.software_renderer && cli.renderer.is_none();
    let app = App::build(&opts)?;
    if let Some((w, h)) = cli.size {
        app.window.window().set_size(slint::LogicalSize::new(w, h));
    }
    match cli.snapshot {
        Some(dir) => kazzu_app::snapshot::run(app, dir, opts.software_renderer),
        None => app.run(),
    }
}

/// Reexecuta o próprio binário com `--renderer software`.
fn relaunch_software() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("kazzu: não foi possível reabrir em modo software: {e}");
            return ExitCode::FAILURE;
        }
    };
    match std::process::Command::new(exe).args(std::env::args().skip(1)).args(["--renderer", "software"]).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("kazzu: não foi possível reabrir em modo software: {e}");
            ExitCode::FAILURE
        }
    }
}

pub(super) fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,lofty=error")).init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(AppError::RetrySoftware) => relaunch_software(),
        Err(e) => {
            eprintln!("kazzu: {e}");
            ExitCode::FAILURE
        }
    }
}
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn main() -> std::process::ExitCode {
    desktop::main()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn main() {
    #[cfg(target_os = "ios")]
    kazzu_app::kazzu_ios_main();
}
