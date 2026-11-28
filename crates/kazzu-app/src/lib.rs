//! Kazzu: interface Slint do player de música.
//!
//! Android e desktop compartilham o backend e usam adaptadores Slint distintos.

pub mod ui {
    slint::include_modules!();
}

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "ios")]
#[path = "ios_platform.rs"]
pub(crate) mod android;
#[cfg(any(target_os = "android", target_os = "ios"))]
mod accessibility;
#[cfg(target_os = "android")]
mod android_paths;
#[cfg(target_os = "android")]
mod android_app;
#[cfg(target_os = "android")]
mod android_view;
#[cfg(target_os = "ios")]
mod ios_app;
#[cfg(target_os = "ios")]
pub use ios_app::{kazzu_ios_main, kazzu_ios_shutdown};
#[cfg(target_os = "ios")]
mod ios_paths;
#[cfg(target_os = "ios")]
mod mobile_view;
#[cfg(any(target_os = "android", target_os = "ios"))]
mod lyrics;
#[cfg(any(target_os = "android", target_os = "ios"))]
mod m3u;
#[cfg(any(target_os = "android", target_os = "ios"))]
mod backup;
#[cfg(any(target_os = "android", target_os = "ios"))]
mod playlist_cover;
mod event_bridge;
#[cfg(all(unix, not(target_os = "android")))]
mod discord_presence;

pub mod accent;
mod accounts;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod app;
pub mod art;
pub mod catalog;
pub mod config;
pub mod controller;
pub mod covers;
#[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "demo"))]
pub mod demo;
pub mod format;
pub mod lru;
pub mod platform;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod slint_view;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod snapshot;
pub mod view;
pub mod vm;
pub mod remote;
pub mod worker;
pub mod ytm_vm;
