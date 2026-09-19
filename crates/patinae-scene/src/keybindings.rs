//! Keyboard binding system
//!
//! This module provides types for defining keyboard shortcuts with optional
//! modifier keys (Ctrl, Shift, Alt).
//!
//! The main types are:
//! - [`KeyBinding`] - Represents a key with optional modifiers (Ctrl, Shift, Alt)
//! - [`KeyBindings`] - Storage for key-to-action mappings

use std::collections::HashMap;

pub use key_code::KeyCode;

mod key_code {
    /// Keyboard key codes used by the input layer.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[allow(dead_code)]
    pub enum KeyCode {
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        Escape,
        Space,
        Enter,
        Backspace,
        Tab,
        Delete,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Home,
        End,
        PageUp,
        PageDown,
        Minus,
        Equal,
        BracketLeft,
        BracketRight,
        Comma,
        Period,
        Slash,
        Backslash,
        Semicolon,
        Quote,
        Backquote,
    }

    impl KeyCode {
        /// Return the US/QWERTY text for printable physical keys used in shortcuts.
        pub const fn canonical_shortcut_text(self, shift: bool) -> Option<&'static str> {
            use KeyCode::*;

            Some(match (self, shift) {
                (Digit0, false) => "0",
                (Digit1, false) => "1",
                (Digit2, false) => "2",
                (Digit3, false) => "3",
                (Digit4, false) => "4",
                (Digit5, false) => "5",
                (Digit6, false) => "6",
                (Digit7, false) => "7",
                (Digit8, false) => "8",
                (Digit9, false) => "9",
                (Digit0, true) => ")",
                (Digit1, true) => "!",
                (Digit2, true) => "@",
                (Digit3, true) => "#",
                (Digit4, true) => "$",
                (Digit5, true) => "%",
                (Digit6, true) => "^",
                (Digit7, true) => "&",
                (Digit8, true) => "*",
                (Digit9, true) => "(",
                (KeyA, false) => "a",
                (KeyB, false) => "b",
                (KeyC, false) => "c",
                (KeyD, false) => "d",
                (KeyE, false) => "e",
                (KeyF, false) => "f",
                (KeyG, false) => "g",
                (KeyH, false) => "h",
                (KeyI, false) => "i",
                (KeyJ, false) => "j",
                (KeyK, false) => "k",
                (KeyL, false) => "l",
                (KeyM, false) => "m",
                (KeyN, false) => "n",
                (KeyO, false) => "o",
                (KeyP, false) => "p",
                (KeyQ, false) => "q",
                (KeyR, false) => "r",
                (KeyS, false) => "s",
                (KeyT, false) => "t",
                (KeyU, false) => "u",
                (KeyV, false) => "v",
                (KeyW, false) => "w",
                (KeyX, false) => "x",
                (KeyY, false) => "y",
                (KeyZ, false) => "z",
                (KeyA, true) => "A",
                (KeyB, true) => "B",
                (KeyC, true) => "C",
                (KeyD, true) => "D",
                (KeyE, true) => "E",
                (KeyF, true) => "F",
                (KeyG, true) => "G",
                (KeyH, true) => "H",
                (KeyI, true) => "I",
                (KeyJ, true) => "J",
                (KeyK, true) => "K",
                (KeyL, true) => "L",
                (KeyM, true) => "M",
                (KeyN, true) => "N",
                (KeyO, true) => "O",
                (KeyP, true) => "P",
                (KeyQ, true) => "Q",
                (KeyR, true) => "R",
                (KeyS, true) => "S",
                (KeyT, true) => "T",
                (KeyU, true) => "U",
                (KeyV, true) => "V",
                (KeyW, true) => "W",
                (KeyX, true) => "X",
                (KeyY, true) => "Y",
                (KeyZ, true) => "Z",
                (Minus, false) => "-",
                (Equal, false) => "=",
                (BracketLeft, false) => "[",
                (BracketRight, false) => "]",
                (Comma, false) => ",",
                (Period, false) => ".",
                (Slash, false) => "/",
                (Backslash, false) => "\\",
                (Semicolon, false) => ";",
                (Quote, false) => "'",
                (Backquote, false) => "`",
                (Minus, true) => "_",
                (Equal, true) => "+",
                (BracketLeft, true) => "{",
                (BracketRight, true) => "}",
                (Comma, true) => "<",
                (Period, true) => ">",
                (Slash, true) => "?",
                (Backslash, true) => "|",
                (Semicolon, true) => ":",
                (Quote, true) => "\"",
                (Backquote, true) => "~",
                _ => return None,
            })
        }
    }
}

/// Represents a keyboard shortcut with optional modifier keys
///
/// A key binding consists of a main key code and optional modifier keys
/// (Ctrl, Shift, Alt). Use the builder-style methods to add modifiers.
///
/// # Examples
///
/// ```ignore
/// use patinae_scene::{KeyBinding, KeyCode};
///
/// // Simple key
/// let key = KeyBinding::new(KeyCode::KeyR);
///
/// // Key with modifiers
/// let ctrl_r = KeyBinding::new(KeyCode::KeyR).ctrl();
/// let ctrl_shift_s = KeyBinding::new(KeyCode::KeyS).ctrl().shift();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    /// The main key code
    pub key: KeyCode,
    /// Whether Ctrl (or Cmd on macOS) must be held
    pub ctrl: bool,
    /// Whether Shift must be held
    pub shift: bool,
    /// Whether Alt (or Option on macOS) must be held
    pub alt: bool,
}

impl KeyBinding {
    /// Create a new key binding for a single key without modifiers
    pub const fn new(key: KeyCode) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Require Ctrl (or Cmd on macOS) to be held
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// Require Shift to be held
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Require Alt (or Option on macOS) to be held
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
}

/// Allow creating a KeyBinding from just a KeyCode for convenience
impl From<KeyCode> for KeyBinding {
    fn from(key: KeyCode) -> Self {
        Self::new(key)
    }
}

/// Parse a key string like `"ctrl+shift+s"` into a [`KeyBinding`].
///
/// Format: `[modifier+]*key` where modifiers are `ctrl`/`cmd`, `shift`,
/// `alt`/`option` (case-insensitive) and key is a letter, digit, function
/// key, or named key.
///
/// # Examples
///
/// ```ignore
/// let binding = parse_key_string("ctrl+s").unwrap();
/// assert!(binding.ctrl);
/// assert_eq!(binding.key, KeyCode::KeyS);
///
/// let binding = parse_key_string("F5").unwrap();
/// assert_eq!(binding.key, KeyCode::F5);
/// ```
pub fn parse_key_string(s: &str) -> Result<KeyBinding, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty key string".into());
    }

    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return Err("empty key string".into());
    }

    let key_part = parts.last().unwrap().trim();
    if key_part.is_empty() {
        return Err("missing key after modifiers".into());
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;

    for &modifier in &parts[..parts.len() - 1] {
        match modifier.trim().to_lowercase().as_str() {
            "ctrl" | "cmd" | "meta" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            other => return Err(format!("unknown modifier: '{}'", other)),
        }
    }

    let key = parse_key_code(key_part)?;

    Ok(KeyBinding {
        key,
        ctrl,
        shift,
        alt,
    })
}

/// Parse a single key name into a [`KeyCode`].
fn parse_key_code(s: &str) -> Result<KeyCode, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        // Letters
        "a" => Ok(KeyCode::KeyA),
        "b" => Ok(KeyCode::KeyB),
        "c" => Ok(KeyCode::KeyC),
        "d" => Ok(KeyCode::KeyD),
        "e" => Ok(KeyCode::KeyE),
        "f" => Ok(KeyCode::KeyF),
        "g" => Ok(KeyCode::KeyG),
        "h" => Ok(KeyCode::KeyH),
        "i" => Ok(KeyCode::KeyI),
        "j" => Ok(KeyCode::KeyJ),
        "k" => Ok(KeyCode::KeyK),
        "l" => Ok(KeyCode::KeyL),
        "m" => Ok(KeyCode::KeyM),
        "n" => Ok(KeyCode::KeyN),
        "o" => Ok(KeyCode::KeyO),
        "p" => Ok(KeyCode::KeyP),
        "q" => Ok(KeyCode::KeyQ),
        "r" => Ok(KeyCode::KeyR),
        "s" => Ok(KeyCode::KeyS),
        "t" => Ok(KeyCode::KeyT),
        "u" => Ok(KeyCode::KeyU),
        "v" => Ok(KeyCode::KeyV),
        "w" => Ok(KeyCode::KeyW),
        "x" => Ok(KeyCode::KeyX),
        "y" => Ok(KeyCode::KeyY),
        "z" => Ok(KeyCode::KeyZ),
        // Digits
        "0" => Ok(KeyCode::Digit0),
        "1" => Ok(KeyCode::Digit1),
        "2" => Ok(KeyCode::Digit2),
        "3" => Ok(KeyCode::Digit3),
        "4" => Ok(KeyCode::Digit4),
        "5" => Ok(KeyCode::Digit5),
        "6" => Ok(KeyCode::Digit6),
        "7" => Ok(KeyCode::Digit7),
        "8" => Ok(KeyCode::Digit8),
        "9" => Ok(KeyCode::Digit9),
        // Function keys
        "f1" => Ok(KeyCode::F1),
        "f2" => Ok(KeyCode::F2),
        "f3" => Ok(KeyCode::F3),
        "f4" => Ok(KeyCode::F4),
        "f5" => Ok(KeyCode::F5),
        "f6" => Ok(KeyCode::F6),
        "f7" => Ok(KeyCode::F7),
        "f8" => Ok(KeyCode::F8),
        "f9" => Ok(KeyCode::F9),
        "f10" => Ok(KeyCode::F10),
        "f11" => Ok(KeyCode::F11),
        "f12" => Ok(KeyCode::F12),
        // Named keys
        "escape" | "esc" => Ok(KeyCode::Escape),
        "space" => Ok(KeyCode::Space),
        "enter" | "return" => Ok(KeyCode::Enter),
        "backspace" => Ok(KeyCode::Backspace),
        "tab" => Ok(KeyCode::Tab),
        "delete" | "del" => Ok(KeyCode::Delete),
        "up" => Ok(KeyCode::ArrowUp),
        "down" => Ok(KeyCode::ArrowDown),
        "left" => Ok(KeyCode::ArrowLeft),
        "right" => Ok(KeyCode::ArrowRight),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" | "page_up" => Ok(KeyCode::PageUp),
        "pagedown" | "page_down" => Ok(KeyCode::PageDown),
        // Punctuation
        "minus" | "-" => Ok(KeyCode::Minus),
        "equal" | "=" => Ok(KeyCode::Equal),
        "bracketleft" | "[" => Ok(KeyCode::BracketLeft),
        "bracketright" | "]" => Ok(KeyCode::BracketRight),
        "comma" | "," => Ok(KeyCode::Comma),
        "period" | "." => Ok(KeyCode::Period),
        "slash" | "/" => Ok(KeyCode::Slash),
        "backslash" | "\\" => Ok(KeyCode::Backslash),
        "semicolon" | ";" => Ok(KeyCode::Semicolon),
        "quote" | "'" => Ok(KeyCode::Quote),
        "backquote" | "`" => Ok(KeyCode::Backquote),
        _ => Err(format!("unknown key: '{}'", s)),
    }
}

/// Storage for keyboard bindings
///
/// A generic container that maps [`KeyBinding`]s to actions of type `A`.
/// This is typically used with closure types for handling key events.
///
/// # Type Parameter
///
/// - `A`: The action type, typically `Arc<dyn Fn(&mut T) + Send + Sync>` for some target type `T`
///
/// # Example
///
/// ```ignore
/// use patinae_scene::{KeyBindings, KeyBinding, KeyCode};
/// use std::sync::Arc;
///
/// let mut bindings: KeyBindings<Arc<dyn Fn() + Send + Sync>> = KeyBindings::new();
/// bindings.bind(KeyCode::KeyR, Arc::new(|| println!("R pressed")));
/// bindings.bind(KeyBinding::new(KeyCode::KeyS).ctrl(), Arc::new(|| println!("Ctrl+S")));
/// ```
#[derive(Debug)]
pub struct KeyBindings<A> {
    bindings: HashMap<KeyBinding, A>,
}

impl<A> Default for KeyBindings<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A> KeyBindings<A> {
    /// Create an empty key bindings container
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Bind a key or key combination to an action
    ///
    /// If the key was already bound, the previous binding is replaced.
    pub fn bind<K: Into<KeyBinding>>(&mut self, key: K, action: A) {
        self.bindings.insert(key.into(), action);
    }

    /// Remove a key binding
    ///
    /// Returns `true` if a binding was removed.
    pub fn unbind<K: Into<KeyBinding>>(&mut self, key: K) -> bool {
        self.bindings.remove(&key.into()).is_some()
    }

    /// Check if a key is bound to an action
    pub fn is_bound<K: Into<KeyBinding>>(&self, key: K) -> bool {
        self.bindings.contains_key(&key.into())
    }

    /// Get the action bound to a key binding
    pub fn get(&self, binding: &KeyBinding) -> Option<&A> {
        self.bindings.get(binding)
    }

    /// Get a mutable reference to the action bound to a key binding
    pub fn get_mut(&mut self, binding: &KeyBinding) -> Option<&mut A> {
        self.bindings.get_mut(binding)
    }

    /// Get the number of bindings
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Check if there are no bindings
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Remove all bindings
    pub fn clear(&mut self) {
        self.bindings.clear();
    }
}

impl<A: Clone> KeyBindings<A> {
    /// Get a clone of the action bound to a key binding
    ///
    /// This is useful when the action needs to be executed while
    /// the bindings container is borrowed.
    pub fn get_cloned(&self, binding: &KeyBinding) -> Option<A> {
        self.bindings.get(binding).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_binding_new() {
        let binding = KeyBinding::new(KeyCode::KeyR);
        assert_eq!(binding.key, KeyCode::KeyR);
        assert!(!binding.ctrl);
        assert!(!binding.shift);
        assert!(!binding.alt);
    }

    #[test]
    fn test_key_binding_modifiers() {
        let binding = KeyBinding::new(KeyCode::KeyS).ctrl().shift();
        assert_eq!(binding.key, KeyCode::KeyS);
        assert!(binding.ctrl);
        assert!(binding.shift);
        assert!(!binding.alt);
    }

    #[test]
    fn test_key_binding_from_keycode() {
        let binding: KeyBinding = KeyCode::KeyA.into();
        assert_eq!(binding.key, KeyCode::KeyA);
        assert!(!binding.ctrl);
        assert!(!binding.shift);
        assert!(!binding.alt);
    }

    #[test]
    fn test_key_binding_equality() {
        let a = KeyBinding::new(KeyCode::KeyR).ctrl();
        let b = KeyBinding::new(KeyCode::KeyR).ctrl();
        let c = KeyBinding::new(KeyCode::KeyR);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_key_bindings_bind_and_get() {
        let mut bindings: KeyBindings<&str> = KeyBindings::new();
        assert!(bindings.is_empty());
        assert_eq!(bindings.len(), 0);
        bindings.bind(KeyCode::KeyR, "reset");
        bindings.bind(KeyBinding::new(KeyCode::KeyS).ctrl(), "save");

        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings.get(&KeyBinding::new(KeyCode::KeyR)),
            Some(&"reset")
        );
        assert_eq!(
            bindings.get(&KeyBinding::new(KeyCode::KeyS).ctrl()),
            Some(&"save")
        );
        assert_eq!(bindings.get(&KeyBinding::new(KeyCode::KeyA)), None);
        assert!(bindings.is_bound(KeyCode::KeyR));
        assert!(!bindings.is_bound(KeyCode::KeyS));
    }

    #[test]
    fn test_key_bindings_unbind() {
        let mut bindings: KeyBindings<i32> = KeyBindings::new();
        bindings.bind(KeyCode::KeyR, 1);
        bindings.bind(KeyCode::KeyS, 2);

        assert!(bindings.unbind(KeyCode::KeyR));
        assert!(!bindings.unbind(KeyCode::KeyR)); // Already removed
        assert_eq!(bindings.len(), 1);
    }

    #[test]
    fn test_key_bindings_clear() {
        let mut bindings: KeyBindings<i32> = KeyBindings::new();
        bindings.bind(KeyCode::KeyR, 1);
        bindings.bind(KeyCode::KeyS, 2);

        bindings.clear();
        assert!(bindings.is_empty());
    }

    #[test]
    fn test_parse_key_string_simple_letter() {
        let b = parse_key_string("s").unwrap();
        assert_eq!(b.key, KeyCode::KeyS);
        assert!(!b.ctrl);
        assert!(!b.shift);
        assert!(!b.alt);
    }

    #[test]
    fn test_parse_key_string_case_insensitive() {
        let b = parse_key_string("S").unwrap();
        assert_eq!(b.key, KeyCode::KeyS);
    }

    #[test]
    fn test_parse_key_string_function_key() {
        let b = parse_key_string("F5").unwrap();
        assert_eq!(b.key, KeyCode::F5);
        assert!(!b.ctrl);
    }

    #[test]
    fn test_parse_key_string_ctrl_modifier() {
        let b = parse_key_string("ctrl+s").unwrap();
        assert_eq!(b.key, KeyCode::KeyS);
        assert!(b.ctrl);
        assert!(!b.shift);
        assert!(!b.alt);
    }

    #[test]
    fn test_parse_key_string_multiple_modifiers() {
        let b = parse_key_string("ctrl+shift+r").unwrap();
        assert_eq!(b.key, KeyCode::KeyR);
        assert!(b.ctrl);
        assert!(b.shift);
        assert!(!b.alt);
    }

    #[test]
    fn test_parse_key_string_all_modifiers() {
        let b = parse_key_string("ctrl+shift+alt+a").unwrap();
        assert_eq!(b.key, KeyCode::KeyA);
        assert!(b.ctrl);
        assert!(b.shift);
        assert!(b.alt);
    }

    #[test]
    fn test_parse_key_string_modifier_aliases() {
        let b = parse_key_string("cmd+option+s").unwrap();
        assert!(b.ctrl); // cmd maps to ctrl
        assert!(b.alt); // option maps to alt
    }

    #[test]
    fn test_parse_key_string_digit() {
        let b = parse_key_string("ctrl+1").unwrap();
        assert_eq!(b.key, KeyCode::Digit1);
        assert!(b.ctrl);
    }

    #[test]
    fn test_canonical_shortcut_text_letters() {
        assert_eq!(KeyCode::KeyA.canonical_shortcut_text(false), Some("a"));
        assert_eq!(KeyCode::KeyA.canonical_shortcut_text(true), Some("A"));
        assert_eq!(KeyCode::KeyZ.canonical_shortcut_text(false), Some("z"));
        assert_eq!(KeyCode::KeyZ.canonical_shortcut_text(true), Some("Z"));
    }

    #[test]
    fn test_canonical_shortcut_text_digits() {
        assert_eq!(KeyCode::Digit1.canonical_shortcut_text(false), Some("1"));
        assert_eq!(KeyCode::Digit1.canonical_shortcut_text(true), Some("!"));
        assert_eq!(KeyCode::Digit0.canonical_shortcut_text(false), Some("0"));
        assert_eq!(KeyCode::Digit0.canonical_shortcut_text(true), Some(")"));
    }

    #[test]
    fn test_canonical_shortcut_text_punctuation() {
        assert_eq!(KeyCode::Minus.canonical_shortcut_text(false), Some("-"));
        assert_eq!(KeyCode::Minus.canonical_shortcut_text(true), Some("_"));
        assert_eq!(KeyCode::Equal.canonical_shortcut_text(false), Some("="));
        assert_eq!(KeyCode::Equal.canonical_shortcut_text(true), Some("+"));
        assert_eq!(
            KeyCode::BracketLeft.canonical_shortcut_text(false),
            Some("[")
        );
        assert_eq!(
            KeyCode::BracketLeft.canonical_shortcut_text(true),
            Some("{")
        );
        assert_eq!(
            KeyCode::Backslash.canonical_shortcut_text(false),
            Some("\\")
        );
        assert_eq!(KeyCode::Backslash.canonical_shortcut_text(true), Some("|"));
        assert_eq!(KeyCode::Quote.canonical_shortcut_text(false), Some("'"));
        assert_eq!(KeyCode::Quote.canonical_shortcut_text(true), Some("\""));
        assert_eq!(KeyCode::Backquote.canonical_shortcut_text(false), Some("`"));
        assert_eq!(KeyCode::Backquote.canonical_shortcut_text(true), Some("~"));
    }

    #[test]
    fn test_canonical_shortcut_text_unsupported_named_keys() {
        assert_eq!(KeyCode::Escape.canonical_shortcut_text(false), None);
        assert_eq!(KeyCode::Enter.canonical_shortcut_text(false), None);
        assert_eq!(KeyCode::ArrowLeft.canonical_shortcut_text(false), None);
        assert_eq!(KeyCode::F1.canonical_shortcut_text(false), None);
    }

    #[test]
    fn test_parse_key_string_named_keys() {
        assert_eq!(parse_key_string("space").unwrap().key, KeyCode::Space);
        assert_eq!(parse_key_string("enter").unwrap().key, KeyCode::Enter);
        assert_eq!(parse_key_string("return").unwrap().key, KeyCode::Enter);
        assert_eq!(parse_key_string("escape").unwrap().key, KeyCode::Escape);
        assert_eq!(parse_key_string("esc").unwrap().key, KeyCode::Escape);
        assert_eq!(parse_key_string("tab").unwrap().key, KeyCode::Tab);
        assert_eq!(parse_key_string("delete").unwrap().key, KeyCode::Delete);
        assert_eq!(parse_key_string("up").unwrap().key, KeyCode::ArrowUp);
        assert_eq!(parse_key_string("down").unwrap().key, KeyCode::ArrowDown);
        assert_eq!(parse_key_string("left").unwrap().key, KeyCode::ArrowLeft);
        assert_eq!(parse_key_string("right").unwrap().key, KeyCode::ArrowRight);
    }

    #[test]
    fn test_parse_key_string_errors() {
        assert!(parse_key_string("").is_err());
        assert!(parse_key_string("ctrl+").is_err());
        assert!(parse_key_string("unknown_key").is_err());
        assert!(parse_key_string("ctrl+badmod+s").is_err());
    }

    #[test]
    fn test_parse_key_string_whitespace() {
        let b = parse_key_string("  ctrl + s  ").unwrap();
        assert_eq!(b.key, KeyCode::KeyS);
        assert!(b.ctrl);
    }

    #[test]
    fn test_key_bindings_modifier_distinction() {
        let mut bindings: KeyBindings<&str> = KeyBindings::new();
        bindings.bind(KeyCode::KeyR, "plain R");
        bindings.bind(KeyBinding::new(KeyCode::KeyR).ctrl(), "Ctrl+R");
        bindings.bind(
            KeyBinding::new(KeyCode::KeyR).ctrl().shift(),
            "Ctrl+Shift+R",
        );

        assert_eq!(bindings.len(), 3);
        assert_eq!(
            bindings.get(&KeyBinding::new(KeyCode::KeyR)),
            Some(&"plain R")
        );
        assert_eq!(
            bindings.get(&KeyBinding::new(KeyCode::KeyR).ctrl()),
            Some(&"Ctrl+R")
        );
        assert_eq!(
            bindings.get(&KeyBinding::new(KeyCode::KeyR).ctrl().shift()),
            Some(&"Ctrl+Shift+R")
        );
    }
}
