use anyhow::{Result, anyhow, bail};
use device_query::Keycode as K;
use enigo::Key as E;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Space,
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    CapsLock,
    Shift,
    LShift,
    RShift,
    Ctrl,
    LCtrl,
    RCtrl,
    Alt,
    LAlt,
    RAlt,
    Meta,
    F(u8),
    Numpad(u8),
    PrintScreen,
    Pause,
}

const NAMES: &[(&str, Key)] = &[
    ("space", Key::Space),
    ("spacebar", Key::Space),
    ("enter", Key::Enter),
    ("return", Key::Enter),
    ("tab", Key::Tab),
    ("esc", Key::Escape),
    ("escape", Key::Escape),
    ("backspace", Key::Backspace),
    ("delete", Key::Delete),
    ("del", Key::Delete),
    ("insert", Key::Insert),
    ("ins", Key::Insert),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pgup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("pgdn", Key::PageDown),
    ("up", Key::Up),
    ("down", Key::Down),
    ("left", Key::Left),
    ("right", Key::Right),
    ("capslock", Key::CapsLock),
    ("caps", Key::CapsLock),
    ("shift", Key::Shift),
    ("lshift", Key::LShift),
    ("leftshift", Key::LShift),
    ("rshift", Key::RShift),
    ("rightshift", Key::RShift),
    ("ctrl", Key::Ctrl),
    ("control", Key::Ctrl),
    ("lctrl", Key::LCtrl),
    ("leftctrl", Key::LCtrl),
    ("rctrl", Key::RCtrl),
    ("rightctrl", Key::RCtrl),
    ("alt", Key::Alt),
    ("option", Key::Alt),
    ("lalt", Key::LAlt),
    ("leftalt", Key::LAlt),
    ("ralt", Key::RAlt),
    ("rightalt", Key::RAlt),
    ("altgr", Key::RAlt),
    ("meta", Key::Meta),
    ("win", Key::Meta),
    ("windows", Key::Meta),
    ("super", Key::Meta),
    ("cmd", Key::Meta),
    ("command", Key::Meta),
    ("printscreen", Key::PrintScreen),
    ("prtsc", Key::PrintScreen),
    ("pause", Key::Pause),
    ("plus", Key::Char('+')),
    ("minus", Key::Char('-')),
    ("comma", Key::Char(',')),
    ("period", Key::Char('.')),
    ("slash", Key::Char('/')),
    ("backslash", Key::Char('\\')),
    ("semicolon", Key::Char(';')),
    ("quote", Key::Char('\'')),
    ("grave", Key::Char('`')),
    ("tilde", Key::Char('`')),
    ("equals", Key::Char('=')),
];

const ENIGO_F: &[E] = &[
    E::F1,
    E::F2,
    E::F3,
    E::F4,
    E::F5,
    E::F6,
    E::F7,
    E::F8,
    E::F9,
    E::F10,
    E::F11,
    E::F12,
    E::F13,
    E::F14,
    E::F15,
    E::F16,
    E::F17,
    E::F18,
    E::F19,
    E::F20,
    #[cfg(not(target_os = "macos"))]
    E::F21,
    #[cfg(not(target_os = "macos"))]
    E::F22,
    #[cfg(not(target_os = "macos"))]
    E::F23,
    #[cfg(not(target_os = "macos"))]
    E::F24,
];

const ENIGO_NUMPAD: [E; 10] = [
    E::Numpad0,
    E::Numpad1,
    E::Numpad2,
    E::Numpad3,
    E::Numpad4,
    E::Numpad5,
    E::Numpad6,
    E::Numpad7,
    E::Numpad8,
    E::Numpad9,
];

const PUNCT: &[(char, K, u16)] = &[
    ('`', K::Grave, 0xC0),
    ('-', K::Minus, 0xBD),
    ('=', K::Equal, 0xBB),
    ('[', K::LeftBracket, 0xDB),
    (']', K::RightBracket, 0xDD),
    ('\\', K::BackSlash, 0xDC),
    (';', K::Semicolon, 0xBA),
    ('\'', K::Apostrophe, 0xDE),
    (',', K::Comma, 0xBC),
    ('.', K::Dot, 0xBE),
    ('/', K::Slash, 0xBF),
];

fn numbered(name: &str) -> Option<Key> {
    let num = |prefix: &str| name.strip_prefix(prefix).and_then(|n| n.parse::<u8>().ok());
    num("f")
        .filter(|n| (1..=24).contains(n))
        .map(Key::F)
        .or_else(|| {
            num("numpad")
                .or_else(|| num("num"))
                .filter(|n| *n <= 9)
                .map(Key::Numpad)
        })
}

#[cfg(target_os = "macos")]
const MAC_KEYCODES: [(char, u16); 47] = [
    ('a', 0),
    ('s', 1),
    ('d', 2),
    ('f', 3),
    ('h', 4),
    ('g', 5),
    ('z', 6),
    ('x', 7),
    ('c', 8),
    ('v', 9),
    ('b', 11),
    ('q', 12),
    ('w', 13),
    ('e', 14),
    ('r', 15),
    ('y', 16),
    ('t', 17),
    ('1', 18),
    ('2', 19),
    ('3', 20),
    ('4', 21),
    ('6', 22),
    ('5', 23),
    ('=', 24),
    ('9', 25),
    ('7', 26),
    ('-', 27),
    ('8', 28),
    ('0', 29),
    (']', 30),
    ('o', 31),
    ('u', 32),
    ('[', 33),
    ('i', 34),
    ('p', 35),
    ('l', 37),
    ('j', 38),
    ('\'', 39),
    ('k', 40),
    (';', 41),
    ('\\', 42),
    (',', 43),
    ('/', 44),
    ('n', 45),
    ('m', 46),
    ('.', 47),
    ('`', 50),
];

#[cfg(target_os = "macos")]
fn char_key(c: char) -> Option<E> {
    let wanted = c.to_ascii_lowercase();
    MAC_KEYCODES
        .iter()
        .find(|(ch, _)| *ch == wanted)
        .map(|(_, code)| E::Other(*code as u32))
}

#[cfg(not(target_os = "macos"))]
fn char_key(c: char) -> Option<E> {
    Some(E::Unicode(c))
}

impl Key {
    pub fn parse(s: &str) -> Result<Key> {
        let raw = s.trim();
        let single = raw.chars().next().filter(|_| raw.chars().count() == 1);
        let name = raw.to_lowercase().replace(['_', '-', ' '], "");
        single
            .map(|c| Key::Char(c.to_lowercase().next().unwrap_or(c)))
            .or_else(|| NAMES.iter().find(|(n, _)| *n == name).map(|(_, k)| *k))
            .or_else(|| numbered(&name))
            .ok_or_else(|| anyhow!("unknown key name {raw:?}"))
    }

    pub fn parse_combo(s: &str) -> Result<Vec<Key>> {
        let s = s.trim();
        match s {
            "" => bail!("empty key combination"),
            "+" => Ok(vec![Key::Char('+')]),
            _ => match s.strip_suffix("++") {
                Some(head) => Ok([Key::parse_combo(head)?, vec![Key::Char('+')]].concat()),
                None => s.split('+').map(Key::parse).collect(),
            },
        }
    }

    pub fn to_enigo(self) -> Result<E> {
        let unsupported = || anyhow!("key {self} is not supported on this platform");
        Ok(match self {
            Key::Char(c) => char_key(c).ok_or_else(unsupported)?,
            Key::Space => E::Space,
            Key::Enter => E::Return,
            Key::Tab => E::Tab,
            Key::Escape => E::Escape,
            Key::Backspace => E::Backspace,
            Key::Delete => E::Delete,
            #[cfg(not(target_os = "macos"))]
            Key::Insert => E::Insert,
            #[cfg(target_os = "macos")]
            Key::Insert => E::Help,
            Key::Home => E::Home,
            Key::End => E::End,
            Key::PageUp => E::PageUp,
            Key::PageDown => E::PageDown,
            Key::Up => E::UpArrow,
            Key::Down => E::DownArrow,
            Key::Left => E::LeftArrow,
            Key::Right => E::RightArrow,
            Key::CapsLock => E::CapsLock,
            Key::Shift => E::Shift,
            Key::LShift => E::LShift,
            Key::RShift => E::RShift,
            Key::Ctrl => E::Control,
            Key::LCtrl => E::LControl,
            Key::RCtrl => E::RControl,
            Key::Alt | Key::RAlt => E::Alt,
            #[cfg(not(target_os = "macos"))]
            Key::LAlt => E::LMenu,
            #[cfg(target_os = "macos")]
            Key::LAlt => E::Option,
            Key::Meta => E::Meta,
            Key::F(n) => *ENIGO_F.get(n as usize - 1).ok_or_else(unsupported)?,
            Key::Numpad(n) => ENIGO_NUMPAD[n as usize],
            #[cfg(not(target_os = "macos"))]
            Key::PrintScreen => E::PrintScr,
            #[cfg(not(target_os = "macos"))]
            Key::Pause => E::Pause,
            #[cfg(target_os = "macos")]
            Key::PrintScreen | Key::Pause => Err(unsupported())?,
        })
    }

    pub fn to_device_query(self) -> Result<Vec<K>> {
        let parse = |s: String| s.parse::<K>().map_err(|e| anyhow!(e));
        let not_hotkey = || anyhow!("key {self} can't be used as a hotkey");
        Ok(match self {
            Key::Char(c) if c.is_ascii_lowercase() => {
                vec![parse(c.to_ascii_uppercase().to_string())?]
            }
            Key::Char(c) if c.is_ascii_digit() => vec![parse(format!("Key{c}"))?],
            Key::Char(c) => vec![
                PUNCT
                    .iter()
                    .find(|p| p.0 == c)
                    .map(|p| p.1)
                    .ok_or_else(not_hotkey)?,
            ],
            Key::Space => vec![K::Space],
            Key::Enter => vec![K::Enter, K::NumpadEnter],
            Key::Tab => vec![K::Tab],
            Key::Escape => vec![K::Escape],
            Key::Backspace => vec![K::Backspace],
            Key::Delete => vec![K::Delete],
            Key::Insert => vec![K::Insert],
            Key::Home => vec![K::Home],
            Key::End => vec![K::End],
            Key::PageUp => vec![K::PageUp],
            Key::PageDown => vec![K::PageDown],
            Key::Up => vec![K::Up],
            Key::Down => vec![K::Down],
            Key::Left => vec![K::Left],
            Key::Right => vec![K::Right],
            Key::CapsLock => vec![K::CapsLock],
            Key::Shift => vec![K::LShift, K::RShift],
            Key::LShift => vec![K::LShift],
            Key::RShift => vec![K::RShift],
            Key::Ctrl => vec![K::LControl, K::RControl],
            Key::LCtrl => vec![K::LControl],
            Key::RCtrl => vec![K::RControl],
            Key::Alt => vec![K::LAlt, K::RAlt, K::LOption, K::ROption],
            Key::LAlt => vec![K::LAlt, K::LOption],
            Key::RAlt => vec![K::RAlt, K::ROption],
            Key::Meta => vec![K::LMeta, K::RMeta, K::Command, K::RCommand],
            Key::F(n) if n <= 20 => vec![parse(format!("F{n}"))?],
            Key::Numpad(n) => vec![parse(format!("Numpad{n}"))?],
            _ => Err(not_hotkey())?,
        })
    }

    pub fn windows_vk(self) -> Option<u16> {
        Some(match self {
            Key::Char(c) if c.is_ascii_lowercase() => c.to_ascii_uppercase() as u16,
            Key::Char(c) if c.is_ascii_digit() => c as u16,
            Key::Char(' ') | Key::Space => 0x20,
            Key::Char(c) => PUNCT.iter().find(|p| p.0 == c)?.2,
            Key::Enter => 0x0D,
            Key::Tab => 0x09,
            Key::Escape => 0x1B,
            Key::Backspace => 0x08,
            Key::Delete => 0x2E,
            Key::Insert => 0x2D,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Left => 0x25,
            Key::Up => 0x26,
            Key::Right => 0x27,
            Key::Down => 0x28,
            Key::CapsLock => 0x14,
            Key::Shift => 0x10,
            Key::LShift => 0xA0,
            Key::RShift => 0xA1,
            Key::Ctrl => 0x11,
            Key::LCtrl => 0xA2,
            Key::RCtrl => 0xA3,
            Key::Alt => 0x12,
            Key::LAlt => 0xA4,
            Key::RAlt => 0xA5,
            Key::Meta => 0x5B,
            Key::F(n) => 0x6F + n as u16,
            Key::Numpad(n) => 0x60 + n as u16,
            Key::PrintScreen => 0x2C,
            Key::Pause => 0x13,
        })
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Numpad(n) => write!(f, "Numpad{n}"),
            other => write!(f, "{other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_character_becomes_a_raw_keycode_and_never_asks_the_layout_service() {
        let code = |c: char| match Key::Char(c).to_enigo().unwrap() {
            E::Other(code) => code,
            other => panic!("{c} mapped to {other:?}, which calls TSMGetInputSourceProperty"),
        };
        assert_eq!(code('a'), 0);
        assert_eq!(code('c'), 8);
        assert_eq!(code('3'), 20);
        assert_eq!(code('C'), 8);
        assert_eq!(code('\''), 39);
        assert!(Key::Char('\u{e9}').to_enigo().is_err());
    }

    #[test]
    fn parse_names() {
        assert_eq!(Key::parse("F").unwrap(), Key::Char('f'));
        assert_eq!(Key::parse("Space").unwrap(), Key::Space);
        assert_eq!(Key::parse("left_ctrl").unwrap(), Key::LCtrl);
        assert_eq!(Key::parse("F12").unwrap(), Key::F(12));
        assert_eq!(Key::parse("numpad5").unwrap(), Key::Numpad(5));
        assert!(Key::parse("F25").is_err());
        assert!(Key::parse("banana").is_err());
    }

    #[test]
    fn combos() {
        assert_eq!(
            Key::parse_combo("ctrl+shift+s").unwrap(),
            vec![Key::Ctrl, Key::Shift, Key::Char('s')]
        );
        assert_eq!(
            Key::parse_combo("ctrl++").unwrap(),
            vec![Key::Ctrl, Key::Char('+')]
        );
        assert_eq!(Key::parse_combo("+").unwrap(), vec![Key::Char('+')]);
        assert!(Key::parse_combo("").is_err());
    }

    #[test]
    fn mappings() {
        assert_eq!(Key::Char('a').to_device_query().unwrap(), vec![K::A]);
        assert_eq!(Key::Char('7').to_device_query().unwrap(), vec![K::Key7]);
        assert!(Key::F(9).to_device_query().is_ok());
        assert_eq!(Key::F(1).windows_vk(), Some(0x70));
        assert!(Key::F(12).to_enigo().is_ok());
    }
}
