use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub fn parse(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in parts.iter() {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" => mods |= Modifiers::ALT,
            "win" | "super" | "meta" | "cmd" => mods |= Modifiers::META,
            _ => {
                if let Some(c) = key_to_code(&lower) {
                    code = Some(c);
                } else {
                    return None;
                }
            }
        }
    }
    let code = code?;
    let modifiers = if mods.is_empty() { None } else { Some(mods) };
    Some(HotKey::new(modifiers, code))
}

fn key_to_code(k: &str) -> Option<Code> {
    // letters
    if k.len() == 1 {
        let ch = k.chars().next().unwrap();
        if ch.is_ascii_lowercase() {
            // map a..z
            let idx = (ch as u8 - b'a') as usize;
            // Code::KeyA .. KeyZ are sequential? assume yes
            let variants = [
                Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE, Code::KeyF,
                Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK, Code::KeyL,
                Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ, Code::KeyR,
                Code::KeyS, Code::KeyT, Code::KeyU, Code::KeyV, Code::KeyW, Code::KeyX,
                Code::KeyY, Code::KeyZ,
            ];
            return Some(variants[idx]);
        }
        if ch.is_ascii_digit() {
            let variants = [
                Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4,
                Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9,
            ];
            let idx = (ch as u8 - b'0') as usize;
            return Some(variants[idx]);
        }
    }
    // function keys
    if let Some(n) = k.strip_prefix('f') {
        if let Ok(n) = n.parse::<u8>() {
            return match n {
                1 => Some(Code::F1), 2 => Some(Code::F2), 3 => Some(Code::F3),
                4 => Some(Code::F4), 5 => Some(Code::F5), 6 => Some(Code::F6),
                7 => Some(Code::F7), 8 => Some(Code::F8), 9 => Some(Code::F9),
                10 => Some(Code::F10), 11 => Some(Code::F11), 12 => Some(Code::F12),
                _ => None,
            };
        }
    }
    match k {
        "space" => Some(Code::Space),
        "enter" | "return" => Some(Code::Enter),
        "escape" | "esc" => Some(Code::Escape),
        "tab" => Some(Code::Tab),
        "backspace" => Some(Code::Backspace),
        "delete" | "del" => Some(Code::Delete),
        _ => None,
    }
}
