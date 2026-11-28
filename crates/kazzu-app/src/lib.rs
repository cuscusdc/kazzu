//! Kazzu: interface Slint do player de música.
//!
//! Android e desktop compartilham o backend e usam adaptadores Slint distintos.

pub mod ui {
    slint::include_modules!();
}

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
mod accessibility;
#[cfg(target_os = "android")]
mod android_paths;
#[cfg(target_os = "android")]
mod android_app;
#[cfg(target_os = "android")]
mod android_view;
#[cfg(target_os = "android")]
mod lyrics;
#[cfg(target_os = "android")]
mod m3u;
#[cfg(target_os = "android")]
mod backup;
#[cfg(target_os = "android")]
mod playlist_cover;
mod event_bridge;
#[cfg(all(unix, not(target_os = "android")))]
mod discord_presence;

pub mod accent;
mod accounts;
#[cfg(not(target_os = "android"))]
pub mod app;
pub mod art;
pub mod catalog;
pub mod config;
pub mod controller;
pub mod covers;
#[cfg(all(not(target_os = "android"), feature = "demo"))]
pub mod demo;
pub mod format;
pub mod lru;
pub mod platform;
#[cfg(not(target_os = "android"))]
pub mod slint_view;
#[cfg(not(target_os = "android"))]
pub mod snapshot;
pub mod view;
pub mod vm;
pub mod remote;
pub mod worker;
pub mod ytm_vm;
