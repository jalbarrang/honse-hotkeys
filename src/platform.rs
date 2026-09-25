//! Key and foreground state, read from Win32.
//!
//! `GetAsyncKeyState` is **global**: it reports a key whether or not this process
//! has focus. That is what makes a hotkey work while the game is not the active
//! window, and also why every caller must gate on [`is_foreground`] — without it a
//! chord fires while the player types in another application.

/// Whether a virtual-key code is currently held.
///
/// Always `false` off Windows.
#[must_use]
pub fn key_down(vk: u32) -> bool {
    #[cfg(windows)]
    {
        // SAFETY: GetAsyncKeyState takes an int and reads global key state; no
        // pointer is involved and every input is valid.
        unsafe { win::GetAsyncKeyState(vk as i32) < 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = vk;
        false
    }
}

/// Whether the foreground window belongs to this process.
///
/// Comparing process ids rather than window handles means the caller does not need
/// to know the game's HWND. A host that draws its overlay into the game's own
/// window stays "foreground" while its menu is open, which is what keeps chords
/// live in that state.
///
/// Always `false` off Windows.
#[must_use]
pub fn is_foreground() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: Win32 foreground/pid queries. A null HWND is checked for before
        // it is passed anywhere.
        unsafe {
            let hwnd = win::GetForegroundWindow();
            if hwnd.is_null() {
                return false;
            }
            let mut pid = 0u32;
            win::GetWindowThreadProcessId(hwnd, &mut pid);
            pid != 0 && pid == win::GetCurrentProcessId()
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    // user32 is not linked by default for a Rust cdylib; kernel32 is.
    #[link(name = "user32")]
    extern "system" {
        pub fn GetAsyncKeyState(v_key: i32) -> i16;
        pub fn GetForegroundWindow() -> *mut c_void;
        pub fn GetWindowThreadProcessId(hwnd: *mut c_void, process_id: *mut u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetCurrentProcessId() -> u32;
    }
}
