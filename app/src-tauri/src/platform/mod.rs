// Camada de plataforma: mesma interface nos dois SOs.
// - secrets: DPAPI (Windows) ⇄ Keychain (macOS)
// - hotkey: hook WH_KEYBOARD_LL (Windows) ⇄ CGEventTap listen-only (macOS)
// - paste: Ctrl+V com VK cru (Windows) ⇄ Cmd+V com kVK_ANSI_V cru (macOS)
// - sidecar: CREATE_NO_WINDOW (Windows) ⇄ Command normal (macOS)

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
