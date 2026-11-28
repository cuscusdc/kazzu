//! Preferências do sistema operacional (movimento reduzido). Melhor esforço: se não der para ler, a chave manual manda.

use crate::controller::SystemPrefs;

/// Linux (GNOME e derivados): `enable-animations`. Outros sistemas: o Slint já cuida do Windows; o resto fica manual.
pub fn system_prefs() -> SystemPrefs {
    SystemPrefs { reduce_motion: read_reduce_motion() }
}

#[cfg(target_os = "linux")]
fn read_reduce_motion() -> Option<bool> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output().ok()?;
    parse_gsettings_bool(&String::from_utf8_lossy(&out.stdout)).map(|animations| !animations)
}

#[cfg(target_os = "android")]
fn read_reduce_motion() -> Option<bool> {
    crate::android::prefers_reduced_motion()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn read_reduce_motion() -> Option<bool> {
    None
}

/// "true\n" / "false\n" -> Some(bool).
pub fn parse_gsettings_bool(text: &str) -> Option<bool> {
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gsettings_output() {
        assert_eq!(parse_gsettings_bool("true\n"), Some(true));
        assert_eq!(parse_gsettings_bool(" false "), Some(false));
        assert_eq!(parse_gsettings_bool(""), None);
        assert_eq!(parse_gsettings_bool("'prefer-dark'"), None);
    }
}

/// Opens a Spotify reference in the system handler. Accept only a Spotify track ID.
pub fn open_spotify_track(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let url = format!("https://open.spotify.com/track/{id}");
    #[cfg(target_os = "android")]
    return crate::android::open_web_url(&url);
    #[cfg(not(target_os = "android"))]
    {
        std::thread::Builder::new().name("kazzu-open-link".into()).spawn(move || {
            #[cfg(target_os = "macos")]
            let (command, args) = ("open", vec![url.as_str()]);
            #[cfg(target_os = "windows")]
            let (command, args) = ("rundll32", vec!["url.dll,FileProtocolHandler", url.as_str()]);
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let (command, args) = ("xdg-open", vec![url.as_str()]);
            let _ = std::process::Command::new(command).args(args).status();
        }).is_ok()
    }
}
