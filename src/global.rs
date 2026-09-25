//! A process-wide registry, for hosts that want one.
//!
//! [`crate::Hotkeys`] is an ordinary value: own it, pass it around, test it. Most
//! plugins want exactly one registry for the life of the process and would rather
//! call free functions, which is what this module provides.
//!
//! Each plugin is loaded as its own dynamic library, so each gets its own copy of
//! these statics, and two plugins using this module do not share a registry.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use crate::{Chord, Handle, Hotkeys, Mods, RegisterError};

fn registry() -> &'static Mutex<Hotkeys> {
    static REGISTRY: OnceLock<Mutex<Hotkeys>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Hotkeys::new()))
}

fn with<R>(f: impl FnOnce(&mut Hotkeys) -> R) -> R {
    let mut guard = match registry().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

pub fn register(
    chord: Chord,
    callback: crate::Callback,
    userdata: *mut c_void,
) -> Result<Handle, RegisterError> {
    with(|hotkeys| hotkeys.register(chord, callback, userdata))
}

pub fn unregister(handle: Handle) -> bool {
    with(|hotkeys| hotkeys.unregister(handle))
}

pub fn set_repeat(handle: Handle, repeat: bool) -> bool {
    with(|hotkeys| hotkeys.set_repeat(handle, repeat))
}

pub fn set_chord(handle: Handle, chord: Chord) -> bool {
    with(|hotkeys| hotkeys.set_chord(handle, chord))
}

pub fn chord_of(handle: Handle) -> Option<Chord> {
    with(|hotkeys| hotkeys.chord_of(handle))
}

pub fn chord_registered(vk: u32, mods: Mods) -> bool {
    with(|hotkeys| hotkeys.chord_registered(vk, mods))
}

pub fn any_chord_uses(mods: Mods) -> bool {
    with(|hotkeys| hotkeys.any_chord_uses(mods))
}

pub fn len() -> usize {
    with(|hotkeys| hotkeys.len())
}

pub fn clear() {
    with(|hotkeys| hotkeys.clear());
}

/// Polls the global registry against real key state. Call once per frame from the
/// host's present callback.
pub fn poll() {
    // The registry lock is held for the whole poll, callbacks included. `std::sync::Mutex`
    // is not reentrant, so a callback that calls back into this module deadlocks.
    // `Hotkeys::poll_with` collects its fire list before invoking anything, which keeps
    // the entry list from being mutated mid-iteration, but that is the only guarantee
    // here.
    with(|hotkeys| hotkeys.poll());
}
