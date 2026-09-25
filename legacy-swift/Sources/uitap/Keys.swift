import CoreGraphics
import Foundation

/// ANSI 虚拟键码。名称大小写不敏感，支持若干别名。
let keyCodes: [String: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17,
    "1": 18, "2": 19, "3": 20, "4": 21, "5": 23, "6": 22, "7": 26, "8": 28, "9": 25, "0": 29,
    "equal": 24, "=": 24, "minus": 27, "-": 27,
    "rightbracket": 30, "]": 30, "leftbracket": 33, "[": 33,
    "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38, "k": 40,
    "return": 36, "enter": 36, "tab": 48, "space": 49, "spacebar": 49,
    "quote": 39, "'": 39, "semicolon": 41, ";": 41, "backslash": 42, "\\": 42,
    "comma": 43, ",": 43, "slash": 44, "/": 44, "period": 47, ".": 47,
    "n": 45, "m": 46, "grave": 50, "`": 50,
    "delete": 51, "backspace": 51, "escape": 53, "esc": 53,
    "right": 124, "left": 123, "down": 125, "up": 126,
    "home": 115, "end": 119, "pageup": 116, "pagedown": 121,
    "forwarddelete": 117, "capslock": 57, "help": 114,
    "f1": 122, "f2": 120, "f3": 99, "f4": 118, "f5": 96, "f6": 97,
    "f7": 98, "f8": 100, "f9": 101, "f10": 109, "f11": 103, "f12": 111,
    "keypad0": 82, "keypad1": 83, "keypad2": 84, "keypad3": 85, "keypad4": 86,
    "keypad5": 87, "keypad6": 88, "keypad7": 89, "keypad8": 91, "keypad9": 92,
    "keypaddecimal": 65, "keypadplus": 69, "keypadminus": 78,
    "keypadmultiply": 67, "keypaddivide": 75, "keypadenter": 76, "keypadclear": 71,
]

let modifierFlags: [String: CGEventFlags] = [
    "cmd": .maskCommand, "command": .maskCommand, "meta": .maskCommand, "⌘": .maskCommand,
    "shift": .maskShift, "⇧": .maskShift,
    "opt": .maskAlternate, "option": .maskAlternate, "alt": .maskAlternate, "⌥": .maskAlternate,
    "ctrl": .maskControl, "control": .maskControl, "⌃": .maskControl,
    "fn": .maskSecondaryFn,
]

/// 解析 `cmd+shift+t` 这类组合键。
func parseCombo(_ combo: String) -> (keyCode: CGKeyCode, flags: CGEventFlags, key: String)? {
    let parts = combo
        .split(separator: "+")
        .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
        .filter { !$0.isEmpty }
    guard !parts.isEmpty else { return nil }

    var flags: CGEventFlags = []
    var keyName: String?
    for part in parts {
        if let flag = modifierFlags[part] {
            flags.insert(flag)
        } else if keyName == nil {
            keyName = part
        } else {
            return nil
        }
    }
    guard let name = keyName, let code = keyCodes[name] else { return nil }
    return (code, flags, name)
}
