# honse-hotkeys

Global hotkey polling for [Hachimi Edge](https://github.com/kairusds/Hachimi-Edge) plugins.

Shared by [honse-pov](../honse-pov) and [honse-tracker](../honse-tracker) so the tricky parts
— edge triggering, the foreground gate, and the AltGr trap below — exist once.

```rust
use honse_hotkeys::{Chord, Hotkeys};

static HOTKEYS: Mutex<Hotkeys> = Mutex::new(Hotkeys::new());

// once, at plugin init
HOTKEYS.lock().unwrap()
    .register(Chord::parse("ctrl+shift+p").unwrap(), toggle, std::ptr::null_mut())
    .unwrap();

// once per frame, from your present callback
HOTKEYS.lock().unwrap().poll();
```

## Why polling instead of a hook

Hachimi's plugin API exposes **no key events**, and Hachimi Edge has **no WndProc plugin
hook**. (The upstream Hachimi fork had one — honse-tracker's original note points at
`hachimi-redux/.../plugin/hotkeys.rs`, which fired from a global WndProc hook. Edge has no
equivalent.) So the only workable route is to read key state directly from Win32 on a
per-frame tick.

That tick comes from the host's present callback, which **the caller registers**. This crate
deliberately does not register it, and has **zero dependencies**: the two consumers carry
different bindings to the plugin API (honse-pov resolves symbols by name at runtime,
honse-tracker goes through its own `edge-sdk`), and depending on either would force the other
to adopt it.

## Why the foreground gate is mandatory

`GetAsyncKeyState` is **global** — it reports a key whether or not your process has focus.
That is what makes a hotkey work while the game is not the active window, and also why a
chord would otherwise fire while the player types in Discord. `poll()` therefore checks that
the foreground window belongs to this process, and while it does not, the registry is frozen
and its edge state reset — so a chord held during a window switch does not fire the moment
focus returns.

The check compares process ids rather than window handles, so the caller never needs the
game's HWND. A host that draws its overlay into the game's own window stays "foreground"
while its menu is open.

## The AltGr trap

`Chord::is_typeable()` reports whether a chord is typed as ordinary text, and it is stricter
than it first looks:

| Chord | Typable | Why |
|---|---|---|
| `p`, `shift+p` | yes | plain characters |
| `ctrl+p`, `alt+p`, `ctrl+shift+p`, `alt+shift+p` | **no** | exactly one of Ctrl or Alt — produces no characters on any layout |
| `ctrl+alt+p`, `ctrl+alt+shift+p` | yes | **Ctrl+Alt is AltGr on Windows** |

AltGr is how everyday characters are typed on non-US layouts — on a Spanish keyboard
`AltGr+2` is `@` — so a Ctrl+Alt chord fires while the player types perfectly ordinary text.
A naive "has a modifier, therefore it is safe" check passes Ctrl+Alt and gets this wrong;
`is_typeable` is `ctrl == alt` for that reason.

The crate **does not refuse** such a chord. Whether to is a product decision — honse-tracker
refuses, because its overlay must never interfere with typing — so the policy stays with the
host:

```rust
if chord.is_typeable() {
    log::error!("refusing {chord:?}: it would fire while typing");
    return;
}
```

## Framing

- **Edge-triggered.** Fires once on the down transition, not every frame while held.
- **Repeat is opt-in** per binding, for nudging something where one press per step would mean
  fifty presses to cross a screen. Never for a toggle — a toggle that repeats flickers.
- **Modifiers match exactly.** `Ctrl+Shift+P` does not also fire while Alt is down. That
  equality is what makes `chord_registered` usable for swallowing key messages: `Ctrl+Shift+J`
  is eaten while a bare `J` passes through to the game untouched.
- **`vk == 0` means unbound** and never matches, so a "not configured" binding cannot
  accidentally match a real key.
- **`Chord::parse` returns `None` for a typo**, so a config file with a mistake leaves the
  previous binding alone instead of silently unbinding it. `"none"` is the explicit way to
  unbind.

## API

Two shapes, same engine:

- `Hotkeys` — an ordinary value. Own it, pass it, test it. `poll_with` takes injected key and
  foreground readers so the logic is testable without Win32, which is how the test suite runs
  anywhere.
- `honse_hotkeys::global` — free functions over a process-wide registry, for hosts that want
  one. Each plugin is its own dynamic library, so each gets its own copy of those statics and
  two plugins cannot interfere.

Chords support `ctrl`/`shift`/`alt` plus `a`-`z`, `0`-`9`, `f1`-`f12`, `space tab insert delete
home end pageup pagedown`, and the punctuation names (`minus`, `equals`, `comma`, `period`,
`slash`, `semicolon`, `quote`, `backquote`, `bracketleft`, `bracketright`, `backslash`).
`describe()` round-trips through `parse()`.

## Tests

```bash
cargo test
```

13 tests cover parsing, round-tripping, exact modifier matching, edge triggering, the repeat
window, the foreground gate (including the hold-across-focus-loss case), rebinding, and the
`chord_registered` / `any_chord_uses` contract that input blocking relies on, plus a doctest of
the usage example.

## Not included

Swallowing the key message so the game does not also see the chord. honse-tracker has that as
`input_block` and it hooks the window procedure, which is a separate concern with a different
failure mode; it is not part of this crate.

## License

GPL-3.0-or-later, matching both consumers.
