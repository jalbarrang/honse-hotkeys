//! Global hotkey polling for Hachimi Edge plugins.
//!
//! # Why polling, not a hook
//!
//! Hachimi's plugin API exposes no key events, and Hachimi Edge has no WndProc plugin
//! hook, so a plugin cannot observe key messages. The upstream Hachimi fork had one:
//! honse-tracker's original note points at `hachimi-redux/.../plugin/hotkeys.rs`,
//! which fired from a global WndProc hook. Edge has no equivalent, so the workable
//! route is to read key state straight from Win32 on a per-frame tick.
//!
//! That tick comes from the host's present callback, which the caller registers. This
//! crate does not register it. Its callers are plugins with different bindings to the
//! plugin API (honse-pov resolves symbols by name at runtime, honse-tracker goes
//! through its own `edge-sdk`), and depending on either would force the other to adopt
//! it. The public surface is therefore `Hotkeys` plus the `platform` readers, and the
//! host supplies the frame tick:
//!
//! ```no_run
//! use honse_hotkeys::{Chord, Hotkeys};
//!
//! static HOTKEYS: std::sync::Mutex<Hotkeys> = std::sync::Mutex::new(Hotkeys::new());
//!
//! extern "C" fn toggle(_userdata: *mut std::ffi::c_void) {
//!     // open/close something
//! }
//!
//! fn setup() {
//!     let mut hotkeys = HOTKEYS.lock().unwrap();
//!     hotkeys.register(Chord::parse("ctrl+shift+p").unwrap(), toggle, std::ptr::null_mut()).unwrap();
//! }
//!
//! /// Called from the host's present callback, once per frame.
//! fn frame() {
//!     HOTKEYS.lock().unwrap().poll();
//! }
//! ```
//!
//! # The typable-chord policy belongs to the host
//!
//! [`Hotkeys::register`] accepts a chord without Ctrl or Alt and does not refuse one.
//! [`Chord::is_typeable`] reports that condition, and whether to enforce it is a
//! product decision: honse-tracker refuses such a chord because its overlay must never
//! interfere with typing, while a plugin binding a key the game ignores may not care.
//!
//! # Behaviour
//!
//! Firing is edge-triggered: once on the down transition, not on every frame while
//! held. Repeat is opt-in per binding, for nudging something where one press per step
//! would mean fifty presses to cross a screen. A toggle must not repeat.
//!
//! While the game is not foreground the registry is frozen and its edge state is reset,
//! so nothing fires in the background. A chord held across a window switch counts as a
//! fresh press when focus returns, and fires once then.

mod chord;
mod platform;

pub mod global;

pub use chord::{key_code, Chord, Mods, VK_CONTROL, VK_MENU, VK_SHIFT};
pub use platform::{is_foreground, key_down};

use std::ffi::c_void;

/// Identifier for a registration. `0` is never returned, so it can mean "failed".
pub type Handle = u64;

/// An `extern "C"` callback, in the shape the plugin APIs use, so callbacks can be
/// passed straight through without an adapter.
pub type Callback = extern "C" fn(userdata: *mut c_void);

/// Frames a repeating chord must be held before it starts repeating, and the gap
/// between repeats after that. Counted in frames rather than milliseconds so the poll
/// needs no clock: at 60fps this is roughly a 0.4s delay then 15 repeats a second,
/// which matches typical key repeat.
pub const REPEAT_DELAY_FRAMES: u32 = 24;
pub const REPEAT_EVERY_FRAMES: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// `vk == 0`: an unbound chord has nothing to match.
    NotBound,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotBound => write!(f, "chord is unbound (vk == 0)"),
        }
    }
}

impl std::error::Error for RegisterError {}

struct Entry {
    handle: Handle,
    chord: Chord,
    callback: Callback,
    userdata: usize,
    repeat: bool,
    was_down: bool,
    held_frames: u32,
}

impl Entry {
    /// Whether this frame fires, advancing the hold counter as a side effect.
    fn tick(&mut self, is_down: bool) -> bool {
        if !is_down {
            self.was_down = false;
            self.held_frames = 0;
            return false;
        }

        let first = !self.was_down;
        self.was_down = true;

        if first {
            self.held_frames = 0;
            return true;
        }

        if !self.repeat {
            return false;
        }

        self.held_frames += 1;
        self.held_frames >= REPEAT_DELAY_FRAMES
            && (self.held_frames - REPEAT_DELAY_FRAMES).is_multiple_of(REPEAT_EVERY_FRAMES)
    }
}

/// A set of chord registrations.
///
/// Hold one per plugin. The type is `Send`, so it can live in a `static Mutex` and be
/// polled from whichever thread the host's frame tick runs on.
#[derive(Default)]
pub struct Hotkeys {
    entries: Vec<Entry>,
    next_handle: Handle,
}

impl Hotkeys {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            // 0 means "failed", so handles start at 1.
            next_handle: 1,
        }
    }

    /// Registers `chord`, replacing any previous binding with the same chord.
    ///
    /// Does not apply the typable-chord policy; see the module docs.
    pub fn register(
        &mut self,
        chord: Chord,
        callback: Callback,
        userdata: *mut c_void,
    ) -> Result<Handle, RegisterError> {
        if !chord.is_bound() {
            return Err(RegisterError::NotBound);
        }

        self.entries.retain(|e| e.chord != chord);

        let handle = self.next_handle;
        self.next_handle += 1;

        self.entries.push(Entry {
            handle,
            chord,
            callback,
            userdata: userdata as usize,
            repeat: false,
            was_down: false,
            held_frames: 0,
        });

        Ok(handle)
    }

    pub fn unregister(&mut self, handle: Handle) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.handle != handle);
        self.entries.len() != before
    }

    /// Makes a chord fire repeatedly while held. Do not use this for a toggle.
    pub fn set_repeat(&mut self, handle: Handle, repeat: bool) -> bool {
        match self.entries.iter_mut().find(|e| e.handle == handle) {
            Some(entry) => {
                entry.repeat = repeat;
                true
            }
            None => false,
        }
    }

    /// Rebinds an existing handle, so the edge state and repeat setting survive.
    pub fn set_chord(&mut self, handle: Handle, chord: Chord) -> bool {
        match self.entries.iter_mut().find(|e| e.handle == handle) {
            Some(entry) => {
                entry.chord = chord;
                entry.was_down = false;
                entry.held_frames = 0;
                true
            }
            None => false,
        }
    }

    #[must_use]
    pub fn chord_of(&self, handle: Handle) -> Option<Chord> {
        self.entries
            .iter()
            .find(|e| e.handle == handle)
            .map(|e| e.chord)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn chords(&self) -> impl Iterator<Item = Chord> + '_ {
        self.entries.iter().map(|e| e.chord)
    }

    /// Whether exactly `(vk, mods)` is registered.
    ///
    /// For a host that wants to swallow key messages its overlay owns: matching
    /// exactly means `Ctrl+Shift+J` is eaten while a bare `J` passes through to the
    /// game untouched.
    #[must_use]
    pub fn chord_registered(&self, vk: u32, mods: Mods) -> bool {
        self.entries
            .iter()
            .any(|e| e.chord.vk == vk && e.chord.mods == mods)
    }

    /// Whether any bound chord uses exactly this modifier set.
    ///
    /// For `WM_CHAR`, where the virtual key is no longer available: if the overlay
    /// owns this modifier combination at all, no character from it should reach a
    /// focused text field.
    #[must_use]
    pub fn any_chord_uses(&self, mods: Mods) -> bool {
        !mods.is_empty()
            && self
                .entries
                .iter()
                .any(|e| e.chord.is_bound() && e.chord.mods == mods)
    }

    /// Polls every registration against real key state. Call once per frame.
    pub fn poll(&mut self) {
        self.poll_with(key_down, is_foreground);
    }

    /// Polls against injected state, so the logic is testable without Win32.
    ///
    /// `key_down` reports whether a virtual key is held; `is_foreground` reports
    /// whether this process owns the foreground window.
    pub fn poll_with(
        &mut self,
        key_down: impl Fn(u32) -> bool,
        is_foreground: impl Fn() -> bool,
    ) {
        if !is_foreground() {
            // Reset edge state so a chord held while unfocused does not fire on
            // refocus.
            for entry in &mut self.entries {
                entry.was_down = false;
                entry.held_frames = 0;
            }
            return;
        }

        // Collect first, then fire: a callback is free to register or unregister,
        // which would otherwise mutate the list mid-iteration.
        let mut to_fire: Vec<(Callback, usize)> = Vec::new();
        for entry in &mut self.entries {
            let is_down = entry.chord.is_down(&key_down);
            if entry.tick(is_down) {
                to_fire.push((entry.callback, entry.userdata));
            }
        }

        for (callback, userdata) in to_fire {
            callback(userdata as *mut c_void);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static HITS: AtomicU32 = AtomicU32::new(0);

    extern "C" fn count(_userdata: *mut c_void) {
        HITS.fetch_add(1, Ordering::Relaxed);
    }

    const CTRL_F1: u32 = 0x70; // VK_F1
    const CTRL_P: u32 = 0x50; // 'P'

    fn held(vks: &'static [u32]) -> impl Fn(u32) -> bool {
        move |vk| vks.contains(&vk)
    }

    fn binding(chord: Chord) -> (Hotkeys, Handle) {
        HITS.store(0, Ordering::Relaxed);
        let mut hotkeys = Hotkeys::new();
        let handle = hotkeys
            .register(chord, count, std::ptr::null_mut())
            .expect("bound chord");
        (hotkeys, handle)
    }

    #[test]
    fn parses_modifiers_and_keys() {
        let chord = Chord::parse("ctrl+shift+p").unwrap();
        assert_eq!(chord.vk, CTRL_P);
        assert!(chord.mods.contains(Mods::CTRL));
        assert!(chord.mods.contains(Mods::SHIFT));
        assert!(!chord.mods.contains(Mods::ALT));

        // Case and spacing are ignored, and a repeated modifier token is harmless.
        let repeated = Chord::parse(" CTRL + Control + f1 ").unwrap();
        assert_eq!(repeated.vk, CTRL_F1);
        assert_eq!(repeated.mods, Mods::CTRL);
        assert_eq!(Chord::parse("alt+space").unwrap().vk, 0x20);
        assert_eq!(Chord::parse("alt+space").unwrap().mods, Mods::ALT);
        assert_eq!(Chord::parse("f12").unwrap().vk, 0x7B);
        assert_eq!(Chord::parse("5").unwrap().vk, 0x35);
    }

    #[test]
    fn rejects_nonsense_without_unbinding() {
        // Callers rely on `None` meaning "typo", so an unparseable string must not
        // come back as an unbound chord.
        assert_eq!(Chord::parse("ctrl+shift+nosuchkey"), None);
        assert_eq!(Chord::parse("ctrl"), None, "modifiers with no key");
        assert_eq!(Chord::parse("a+b"), None, "two primary keys");
        assert_eq!(Chord::parse("f13"), None);

        // "none" is the explicit way to unbind.
        assert_eq!(Chord::parse("none"), Some(Chord::NONE));
        assert_eq!(Chord::parse("  "), Some(Chord::NONE));
    }

    #[test]
    fn typable_policy_inputs() {
        // Ctrl or Alt alone, with or without Shift, types nothing on any layout.
        assert!(!Chord::parse("ctrl+p").unwrap().is_typeable());
        assert!(!Chord::parse("alt+p").unwrap().is_typeable());
        assert!(!Chord::parse("ctrl+shift+p").unwrap().is_typeable());
        assert!(!Chord::parse("alt+shift+p").unwrap().is_typeable());

        // Plain text.
        assert!(Chord::parse("p").unwrap().is_typeable());
        assert!(Chord::parse("shift+p").unwrap().is_typeable());

        // AltGr: Ctrl+Alt types text, with or without Shift.
        assert!(Chord::parse("ctrl+alt+p").unwrap().is_typeable());
        assert!(Chord::parse("ctrl+alt+shift+p").unwrap().is_typeable());
    }

    #[test]
    fn modifiers_match_exactly() {
        let chord = Chord::new(Mods::CTRL | Mods::SHIFT, CTRL_P);
        assert!(chord.is_down(&held(&[CTRL_P, VK_CONTROL, VK_SHIFT])));
        // Extra Alt means this is a different chord, not a superset match.
        assert!(!chord.is_down(&held(&[CTRL_P, VK_CONTROL, VK_SHIFT, VK_MENU])));
        // Missing Shift.
        assert!(!chord.is_down(&held(&[CTRL_P, VK_CONTROL])));
        // Key not held.
        assert!(!chord.is_down(&held(&[VK_CONTROL, VK_SHIFT])));
        // Unbound never matches.
        assert!(!Chord::NONE.is_down(&held(&[CTRL_P])));
    }

    #[test]
    fn describe_round_trips() {
        for text in ["ctrl+shift+p", "alt+f1", "ctrl+shift+space", "ctrl+minus"] {
            let chord = Chord::parse(text).unwrap();
            let described = chord.describe();
            assert_eq!(
                Chord::parse(&described).unwrap(),
                chord,
                "{text} described as {described}"
            );
        }
        assert_eq!(Chord::NONE.describe(), "none");
    }

    #[test]
    fn fires_once_on_down_transition() {
        let (mut hotkeys, _) = binding(Chord::new(Mods::CTRL, CTRL_F1));
        let down = held(&[CTRL_F1, VK_CONTROL]);
        let up = held(&[]);

        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1, "down transition fires");

        hotkeys.poll_with(&down, || true);
        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1, "held does not re-fire");

        hotkeys.poll_with(&up, || true);
        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 2, "re-press fires again");
    }

    #[test]
    fn repeat_is_opt_in() {
        let (mut hotkeys, handle) = binding(Chord::new(Mods::CTRL, CTRL_F1));
        let down = held(&[CTRL_F1, VK_CONTROL]);

        assert!(hotkeys.set_repeat(handle, true));

        // First frame fires; then nothing until the delay elapses.
        for _ in 0..REPEAT_DELAY_FRAMES {
            hotkeys.poll_with(&down, || true);
        }
        assert_eq!(HITS.load(Ordering::Relaxed), 1, "still within the delay");

        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 2, "repeat begins after the delay");
    }

    #[test]
    fn foreground_gate_freezes_and_resets() {
        let (mut hotkeys, _) = binding(Chord::new(Mods::CTRL, CTRL_F1));
        let down = held(&[CTRL_F1, VK_CONTROL]);

        hotkeys.poll_with(&down, || false);
        assert_eq!(HITS.load(Ordering::Relaxed), 0, "not foreground: no fire");

        hotkeys.poll_with(&down, || true);
        assert_eq!(
            HITS.load(Ordering::Relaxed),
            1,
            "a chord held while unfocused fires once on refocus, as a fresh press"
        );
    }

    #[test]
    fn unregister_stops_firing() {
        let (mut hotkeys, handle) = binding(Chord::new(Mods::CTRL, CTRL_F1));
        let down = held(&[CTRL_F1, VK_CONTROL]);

        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1);
        hotkeys.poll_with(held(&[]), || true);

        assert!(hotkeys.unregister(handle));
        assert!(!hotkeys.unregister(handle), "second unregister is a no-op");

        hotkeys.poll_with(&down, || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1, "gone after unregister");
        assert!(hotkeys.is_empty());
    }

    #[test]
    fn unbound_chord_is_refused() {
        let mut hotkeys = Hotkeys::new();
        assert_eq!(
            hotkeys.register(Chord::NONE, count, std::ptr::null_mut()),
            Err(RegisterError::NotBound)
        );
        assert!(hotkeys.is_empty());
    }

    #[test]
    fn registering_same_chord_replaces() {
        let mut hotkeys = Hotkeys::new();
        let chord = Chord::new(Mods::CTRL, CTRL_F1);
        hotkeys.register(chord, count, std::ptr::null_mut()).unwrap();
        hotkeys.register(chord, count, std::ptr::null_mut()).unwrap();
        assert_eq!(hotkeys.len(), 1, "one binding per chord");
    }

    #[test]
    fn queries_match_the_input_block_contract() {
        let (hotkeys, _) = binding(Chord::new(Mods::CTRL | Mods::SHIFT, CTRL_P));

        assert!(hotkeys.chord_registered(CTRL_P, Mods::CTRL | Mods::SHIFT));
        // A bare P is not ours, so it must reach the game.
        assert!(!hotkeys.chord_registered(CTRL_P, Mods::NONE));
        assert!(hotkeys.any_chord_uses(Mods::CTRL | Mods::SHIFT));
        assert!(!hotkeys.any_chord_uses(Mods::ALT));
        assert!(!hotkeys.any_chord_uses(Mods::NONE), "no modifiers is never owned");

        assert_eq!(hotkeys.chord_of(1), Some(Chord::new(Mods::CTRL | Mods::SHIFT, CTRL_P)));
        assert_eq!(hotkeys.chord_of(99), None);
    }

    #[test]
    fn set_chord_rebinds_without_losing_handles() {
        let (mut hotkeys, handle) = binding(Chord::new(Mods::CTRL, CTRL_F1));
        assert!(hotkeys.set_chord(handle, Chord::new(Mods::ALT, 0x20)));
        assert_eq!(hotkeys.chord_of(handle), Some(Chord::new(Mods::ALT, 0x20)));

        HITS.store(0, Ordering::Relaxed);
        hotkeys.poll_with(held(&[0x20, VK_MENU]), || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1);

        // The old chord is no longer bound to this handle.
        hotkeys.poll_with(held(&[]), || true);
        hotkeys.poll_with(held(&[CTRL_F1, VK_CONTROL]), || true);
        assert_eq!(HITS.load(Ordering::Relaxed), 1);
    }
}
