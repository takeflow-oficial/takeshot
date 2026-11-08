//! Operating system and data: screen capture, clipboard and files, fonts, saved settings, and the
//! resident's tray, hotkeys, single instance and start with the system.

pub mod actions;
pub mod autostart;
pub mod borderless;
pub mod capture;
pub mod fonts;
pub mod headless;
pub mod hotkeys;
pub mod instance;
pub mod library;
#[cfg(target_os = "macos")]
pub mod mac_titlebar;
pub mod notify;
pub mod prtsc_hook;
pub mod recents;
pub mod resident;
pub mod settings;
pub mod thumbcache;
pub mod tray;
