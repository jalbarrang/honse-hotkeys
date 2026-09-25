//! Chords: a modifier set plus one primary virtual-key code.

use std::ops::BitOr;

/// Virtual-key codes used for the modifiers. The letters and digits share their
/// ASCII values (`VK_A` is 0x41), which is why the parser needs no table for them.
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;

/// A set of held modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(1 << 0);
    pub const SHIFT: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Masks off anything outside the three known modifiers.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & 0b111)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Mods {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// A modifier set plus a primary key.
///
/// `vk == 0` means unbound and never matches, so a "not configured" binding cannot
/// accidentally match a real key. This is the convention honse-tracker used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub vk: u32,
}

impl Chord {
    pub const NONE: Self = Self {
        mods: Mods::NONE,
        vk: 0,
    };

    #[must_use]
    pub const fn new(mods: Mods, vk: u32) -> Self {
        Self { mods, vk }
    }

    #[must_use]
    pub const fn is_bound(self) -> bool {
        self.vk != 0
    }

    /// Whether the chord can be typed as ordinary text.
    ///
    /// A chord is safe to use globally only when it holds exactly one of Ctrl or Alt:
    ///
    /// - neither: the chord is plain characters;
    /// - exactly one of Ctrl or Alt, with or without Shift: no layout produces
    ///   characters from it;
    /// - Ctrl and Alt together: on Windows that is AltGr, which is how non-US layouts
    ///   type everyday characters (Spanish `AltGr+2` is `@`), and Shift takes part too,
    ///   so these chords fire during ordinary typing.
    ///
    /// A test like `!ctrl && !alt` is therefore not enough, since it passes Ctrl+Alt.
    ///
    /// This crate does not refuse such a chord; the policy belongs to the host. Check
    /// this before [`crate::Hotkeys::register`] if you want to refuse it.
    #[must_use]
    pub const fn is_typeable(self) -> bool {
        let ctrl = self.mods.contains(Mods::CTRL);
        let alt = self.mods.contains(Mods::ALT);
        ctrl == alt
    }

    /// Reads the currently held modifiers from a key-state reader.
    #[must_use]
    pub fn mods_from(key_down: &dyn Fn(u32) -> bool) -> Mods {
        let mut mods = Mods::NONE;
        if key_down(VK_CONTROL) {
            mods = mods | Mods::CTRL;
        }
        if key_down(VK_SHIFT) {
            mods = mods | Mods::SHIFT;
        }
        if key_down(VK_MENU) {
            mods = mods | Mods::ALT;
        }
        mods
    }

    /// Whether this chord is held, given a key-state reader.
    ///
    /// Modifiers are compared for equality, not as a superset, so `Ctrl+Shift+P` does
    /// not also fire while Alt is down. [`crate::Hotkeys::chord_registered`] relies on
    /// that to decide whether a key message should be swallowed.
    #[must_use]
    pub fn is_down(self, key_down: &dyn Fn(u32) -> bool) -> bool {
        self.is_bound() && key_down(self.vk) && Self::mods_from(key_down) == self.mods
    }

    /// Parses `"ctrl+shift+p"`. `"none"` (or an empty string) parses to an unbound
    /// chord, which is how a config file switches a binding off.
    ///
    /// Returns `None` for anything unrecognised, so a typo leaves the previous
    /// binding alone instead of silently unbinding it.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("none") {
            return Some(Self::NONE);
        }

        let mut chord = Self::NONE;
        let mut key_seen = false;

        for token in text.split('+') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }

            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.mods = chord.mods | Mods::CTRL,
                "shift" => chord.mods = chord.mods | Mods::SHIFT,
                "alt" | "menu" => chord.mods = chord.mods | Mods::ALT,
                name => {
                    if key_seen {
                        return None; // two primary keys
                    }
                    chord.vk = key_code(name)?;
                    key_seen = true;
                }
            }
        }

        if !chord.is_bound() {
            return None;
        }
        Some(chord)
    }

    /// `"ctrl+shift+vk50"`, using the same vocabulary [`Self::parse`] accepts.
    #[must_use]
    pub fn describe(self) -> String {
        if !self.is_bound() {
            return "none".to_owned();
        }

        let mut parts = Vec::new();
        if self.mods.contains(Mods::CTRL) {
            parts.push("ctrl".to_owned());
        }
        if self.mods.contains(Mods::SHIFT) {
            parts.push("shift".to_owned());
        }
        if self.mods.contains(Mods::ALT) {
            parts.push("alt".to_owned());
        }
        parts.push(key_name(self.vk));
        parts.join("+")
    }
}

/// Maps a single token from a chord string to a virtual-key code.
#[must_use]
pub fn key_code(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0].to_ascii_uppercase();
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(u32::from(c));
        }
    }

    if let Some(digits) = name.strip_prefix('f') {
        if let Ok(n) = digits.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some(0x70 + n - 1); // VK_F1 = 0x70
            }
        }
    }

    match name {
        "space" => Some(0x20),
        "tab" => Some(0x09),
        "insert" => Some(0x2D),
        "delete" | "del" => Some(0x2E),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "backquote" => Some(0xC0),
        "minus" => Some(0xBD),
        "equals" => Some(0xBB),
        "comma" => Some(0xBC),
        "period" => Some(0xBE),
        "slash" => Some(0xBF),
        "semicolon" => Some(0xBA),
        "quote" => Some(0xDE),
        "bracketleft" => Some(0xDB),
        "bracketright" => Some(0xDD),
        "backslash" => Some(0xDC),
        _ => None,
    }
}

fn key_name(vk: u32) -> String {
    let letter_or_digit = u8::try_from(vk)
        .ok()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if let Some(c) = letter_or_digit {
        return (c as char).to_ascii_lowercase().to_string();
    }

    if (0x70..=0x7B).contains(&vk) {
        return format!("f{}", vk - 0x70 + 1);
    }

    let named = match vk {
        0x20 => "space",
        0x09 => "tab",
        0x2D => "insert",
        0x2E => "delete",
        0x24 => "home",
        0x23 => "end",
        0x21 => "pageup",
        0x22 => "pagedown",
        0xC0 => "backquote",
        0xBD => "minus",
        0xBB => "equals",
        0xBC => "comma",
        0xBE => "period",
        0xBF => "slash",
        0xBA => "semicolon",
        0xDE => "quote",
        0xDB => "bracketleft",
        0xDD => "bracketright",
        0xDC => "backslash",
        _ => return format!("vk{vk:02x}"),
    };
    named.to_owned()
}
