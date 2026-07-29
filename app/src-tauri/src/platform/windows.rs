use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{handle_key, Cmd, HookState, Settings, Shared};

// --- DPAPI: chaves criptografadas por usuário do Windows no settings.json ---
mod dpapi {
    use base64::Engine;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    fn b64() -> base64::engine::GeneralPurpose {
        base64::engine::general_purpose::STANDARD
    }

    pub fn protect(plain: &str) -> Option<String> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: plain.len() as u32,
                pbData: plain.as_ptr() as *mut u8,
            };
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            if CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            ) == 0
            {
                return None;
            }
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            Some(b64().encode(bytes))
        }
    }

    pub fn unprotect(encoded: &str) -> Option<String> {
        let bytes = b64().decode(encoded).ok()?;
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: bytes.len() as u32,
                pbData: bytes.as_ptr() as *mut u8,
            };
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            if CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            ) == 0
            {
                return None;
            }
            let plain = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            String::from_utf8(plain).ok()
        }
    }
}

/// DPAPI não precisa de nome de item: o blob é autossuficiente.
pub fn protect_secret(_name: &str, plain: &str) -> Option<String> {
    dpapi::protect(plain)
}

pub fn unprotect_secret(_name: &str, stored: &str) -> Option<String> {
    dpapi::unprotect(stored)
}

// --- Tabela de teclas (VK) ---
pub fn modifier_group(name: &str) -> Option<Vec<u32>> {
    match name {
        "CTRL" | "CONTROL" => Some(vec![0xA2, 0xA3]), // L/RCONTROL
        "WIN" | "META" | "SUPER" | "CMD" | "COMMAND" => Some(vec![0x5B, 0x5C]), // L/RWIN
        "ALT" | "OPTION" | "OPT" => Some(vec![0xA4, 0xA5]), // LMENU/RMENU (AltGr = RMENU)
        "SHIFT" => Some(vec![0xA0, 0xA1]),            // L/RSHIFT
        _ => None,
    }
}

pub fn key_from_name(name: &str) -> u32 {
    match name {
        "F1" => 0x70, "F2" => 0x71, "F3" => 0x72, "F4" => 0x73, "F5" => 0x74, "F6" => 0x75,
        "F7" => 0x76, "F8" => 0x77, "F10" => 0x79, "F11" => 0x7A, "F12" => 0x7B,
        "SCROLLLOCK" => 0x91, "PAUSE" => 0x13, "CAPSLOCK" => 0x14,
        "INSERT" => 0x2D, "HOME" => 0x24, "END" => 0x23,
        "SPACE" => 0x20, "ESC" | "ESCAPE" => 0x1B, "BACKSPACE" => 0x08, "TAB" => 0x09,
        _ => 0x78, // F9
    }
}

// --- Hook WH_KEYBOARD_LL próprio: só rastreia keydown/keyup de VKs. O rdev chamava
// ToUnicode dentro do hook, o que consome dead keys (~/^) do sistema inteiro. ---
thread_local! {
    static HOOK_STATE: std::cell::RefCell<Option<HookState>> = const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
        WM_SYSKEYUP,
    };
    if code >= 0 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        // ignora teclas injetadas (o próprio Ctrl+V sintético não polui o estado)
        if kb.flags & LLKHF_INJECTED == 0 {
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            if down || msg == WM_KEYUP || msg == WM_SYSKEYUP {
                HOOK_STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        handle_key(st, kb.vkCode, down);
                    }
                });
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

pub fn spawn_hotkey_listener(shared: Arc<Shared>, tx: Sender<Cmd>) {
    std::thread::spawn(move || unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetMessageW, SetWindowsHookExW, MSG, WH_KEYBOARD_LL,
        };
        HOOK_STATE.with(|s| {
            *s.borrow_mut() = Some(HookState {
                shared,
                tx,
                pressed: HashSet::new(),
                active: false,
                recording: false,
            });
        });
        println!("[hotkey] instalando hook global...");
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), std::ptr::null_mut(), 0);
        if hook.is_null() {
            eprintln!("[hotkey] SetWindowsHookExW falhou");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
    });
}

// --- Inserção ---

/// espera Shift/Ctrl/Alt/Win físicos soltarem: modificador físico + V sintético
/// formaria combos do sistema (Ctrl+Shift troca layout de teclado, Win+V etc.)
pub fn wait_modifiers_released(timeout: Duration) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline
        && [0x10i32, 0x11, 0x12, 0x5B, 0x5C]
            .iter()
            .any(|&vk| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0)
    {
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub fn paste_shortcut() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard};
    let mut enigo = Enigo::new(&enigo::Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
    enigo.key(Key::Other(0x56), Direction::Click).map_err(|e| e.to_string())?; // VK_V cru, independe de layout
    enigo.key(Key::Control, Direction::Release).map_err(|e| e.to_string())?;
    Ok(())
}

// --- Sidecar ---
pub fn spawn_sidecar(s: &Settings) -> Option<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !std::path::Path::new(&s.python).exists() || !std::path::Path::new(&s.sidecar).exists() {
        println!("[sidecar] transcrição local não configurada — operando só em nuvem (Groq)");
        return None;
    }
    match std::process::Command::new(&s.python)
        .arg(&s.sidecar)
        .env("OPENFLOW_STT_PORT", s.stt_port.to_string())
        .env("OPENFLOW_PARENT_PID", std::process::id().to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("[sidecar] falha ao iniciar ({e}) — inicie manualmente: {} {}", s.python, s.sidecar);
            None
        }
    }
}
