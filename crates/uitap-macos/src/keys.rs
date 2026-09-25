//! ANSI 虚拟键码与组合键解析。键位表与 Swift 版逐项一致。

use uitap_core::backend::Modifier;

pub fn key_code(name: &str) -> Option<u16> {
    let lowered = name.to_ascii_lowercase();
    match lowered.as_str() {
        "a" => Some(0),
        "s" => Some(1),
        "d" => Some(2),
        "f" => Some(3),
        "h" => Some(4),
        "g" => Some(5),
        "z" => Some(6),
        "x" => Some(7),
        "c" => Some(8),
        "v" => Some(9),
        "b" => Some(11),
        "q" => Some(12),
        "w" => Some(13),
        "e" => Some(14),
        "r" => Some(15),
        "y" => Some(16),
        "t" => Some(17),
        "1" => Some(18),
        "2" => Some(19),
        "3" => Some(20),
        "4" => Some(21),
        "6" => Some(22),
        "5" => Some(23),
        "equal" | "=" => Some(24),
        "9" => Some(25),
        "7" => Some(26),
        "minus" | "-" => Some(27),
        "8" => Some(28),
        "0" => Some(29),
        "rightbracket" | "]" => Some(30),
        "o" => Some(31),
        "u" => Some(32),
        "leftbracket" | "[" => Some(33),
        "i" => Some(34),
        "p" => Some(35),
        "return" | "enter" => Some(36),
        "l" => Some(37),
        "j" => Some(38),
        "quote" | "'" => Some(39),
        "k" => Some(40),
        "semicolon" | ";" => Some(41),
        "backslash" | "\\" => Some(42),
        "comma" | "," => Some(43),
        "slash" | "/" => Some(44),
        "n" => Some(45),
        "m" => Some(46),
        "period" | "." => Some(47),
        "tab" => Some(48),
        "space" | "spacebar" => Some(49),
        "grave" | "`" => Some(50),
        "delete" | "backspace" => Some(51),
        "escape" | "esc" => Some(53),
        "capslock" => Some(57),
        "keypaddecimal" => Some(65),
        "keypadmultiply" => Some(67),
        "keypadplus" => Some(69),
        "keypadclear" => Some(71),
        "keypaddivide" => Some(75),
        "keypadenter" => Some(76),
        "keypadminus" => Some(78),
        "keypad0" => Some(82),
        "keypad1" => Some(83),
        "keypad2" => Some(84),
        "keypad3" => Some(85),
        "keypad4" => Some(86),
        "keypad5" => Some(87),
        "keypad6" => Some(88),
        "keypad7" => Some(89),
        "keypad8" => Some(91),
        "keypad9" => Some(92),
        "f5" => Some(96),
        "f6" => Some(97),
        "f7" => Some(98),
        "f3" => Some(99),
        "f8" => Some(100),
        "f9" => Some(101),
        "f11" => Some(103),
        "f10" => Some(109),
        "f12" => Some(111),
        "help" => Some(114),
        "home" => Some(115),
        "pageup" => Some(116),
        "forwarddelete" => Some(117),
        "f4" => Some(118),
        "end" => Some(119),
        "f2" => Some(120),
        "pagedown" => Some(121),
        "f1" => Some(122),
        "left" => Some(123),
        "right" => Some(124),
        "down" => Some(125),
        "up" => Some(126),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Combo {
    pub key_code: u16,
    pub modifiers: Vec<Modifier>,
    pub key: String,
}

/// 解析 `cmd+shift+t` 这类组合键。
pub fn parse_combo(text: &str) -> Option<Combo> {
    let mut modifiers: Vec<Modifier> = Vec::new();
    let mut key_name: Option<String> = None;

    for part in text.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(m) = Modifier::parse(part) {
            if !modifiers.contains(&m) {
                modifiers.push(m);
            }
            continue;
        }
        if key_name.is_some() {
            return None;
        }
        key_name = Some(part.to_ascii_lowercase());
    }

    let key = key_name?;
    let key_code = key_code(&key)?;
    Some(Combo {
        key_code,
        modifiers,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_letters_and_chords() {
        assert_eq!(key_code("a"), Some(0));
        assert_eq!(key_code("A"), Some(0));

        let combo = parse_combo("cmd+shift+t").unwrap();
        assert_eq!(combo.key_code, 17);
        assert_eq!(combo.modifiers, vec![Modifier::Cmd, Modifier::Shift]);
    }

    #[test]
    fn parses_named_keys() {
        assert_eq!(parse_combo("esc").unwrap().key_code, 53);
        assert_eq!(parse_combo("return").unwrap().key_code, 36);
        assert!(parse_combo("nope").is_none());
    }

    #[test]
    fn rejects_two_plain_keys() {
        assert!(parse_combo("a+b").is_none());
    }
}
