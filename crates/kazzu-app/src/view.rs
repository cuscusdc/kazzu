//! Fronteira entre a lógica (controller) e a tela. A implementação real (`SlintView`) escreve em `AppState`/`Theme`;
//! nos testes entra um `View` que só grava as chamadas, então nada aqui exige janela.

use crate::accent::Accent;
use crate::vm::*;

pub trait View {
    fn set_playback(&self, p: &PlaybackVm);
    /// `progress` 0..1.
    fn set_seek_available(&self, _available: bool) {}
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_position_millis(&self, _position: u64, _duration: Option<u64>) {}
    fn set_progress(&self, progress: f32, position: &str, remaining: &str);
    /// Capa grande, fundo do palco e acento da faixa atual.
    fn set_now_art(&self, art: &NowArtVm);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_lyrics(&self, _lines: &[(i32, String)]) {}
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_remote_lyrics(&self, _text: &str, _attribution: &str, _loading: bool) {}
    /// Palco sem faixa tocando: capa pré-desfocada de uma faixa da biblioteca (ou nada).
    fn set_ambient(&self, backdrop: Option<&crate::art::Pixels>);
    /// Acento da capa nos dois temas; `dynamic` = "Cor de destaque: Da capa". Também escreve as cores do palco.
    fn set_accent(&self, accent: &Accent, dynamic: bool);
    fn set_home(&self, home: &HomeVm);
    fn set_explore(&self, explore: &ExploreVm);
    fn set_library(&self, library: &LibraryVm);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_listening_stats(&self, _stats: &kazzu_library::ListeningStats) {}
    fn set_settings(&self, settings: &SettingsVm, providers: &[ProviderVm]);
    fn import_status(&self, _pending: bool, _message: &str) {}
    fn set_loading(&self, loading: bool);
    fn set_folder_listing(&self, _listing: Result<&FolderListingVm, &str>) {}
    fn set_queue(&self, queue: &QueueVm);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_queue_ids(&self, _ids: &[kazzu_core::QueueItemId]) {}
    fn set_search(&self, search: &SearchVm);
    fn restore_focus(&self, _key: &str) {}
    fn detail_scroll(&self) -> f32 { 0.0 }
    fn restore_detail_scroll(&self, _position: f32) {}
    fn set_detail(&self, detail: &DetailVm);
    fn open_detail(&self, open: bool);
    fn set_scan(&self, scan: Option<&ScanVm>);
    /// Aviso temporário. `action` = rótulo do botão ("Desfazer", "Tentar de novo"), vazio = sem ação.
    fn toast(&self, text: &str, error: bool, action: &str);
    /// Marca qual linha está tocando (em todas as listas visíveis).
    fn mark_playing(&self, id: Option<&str>);
    fn mark_liked(&self, id: &str, liked: bool);
    fn set_prefs(&self, prefs: &PrefsVm);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_mobile_appearance(&self, _appearance: &crate::config::MobileAppearance) {}
    fn set_volume(&self, volume: f32);
    fn sleep_timer_elapsed(&self) {}
    /// Há conexão (ou ao menos a última chamada de rede deu certo).
    fn set_online(&self, online: bool);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn set_storage(&self, _files: u64, _cache: u64) {}
    /// Oferta "Deixar mais leve?" (quadros lentos com vidro ligado).
    fn set_perf_offer(&self, offer: bool);
    /// Abre a folha "Nova playlist" / "Adicionar à playlist" etc. quando o controller precisa fechá-las.
    fn close_sheets(&self);
}
