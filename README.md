# tf2-input

Query Titanfall 2 input **by action, not by hardcoded key**, from a native [rrplug](https://github.com/R2NorthstarTools/rrplug) plugin.

A player can bind an action to several keys (the keybind menu allows a primary + alternate, and people add more), so hardcoding `KEY_SPACE` / `KEY_LCONTROL` breaks for anyone who rebinds. This crate reads the engine's key-bind table — the same one the game reads — resolves each [`Input`] action to the set of `ButtonCode_t`s currently bound to it, and tracks which are physically held.

```rust
use tf2_input::Input;

// in your PostEvent / input hook, for every button press/release:
tf2_input::on_button_event(scan, pressed);

// on the engine thread (runframe), once + periodically, to pick up live rebinds:
tf2_input::refresh();

// then query by action:
if tf2_input::is_down(Input::Jump) { /* ... */ }
let is_crouch_key = tf2_input::matches(Input::Crouch, scan);
```

## Why

- **Works for any rebind** — including mouse buttons or scroll-wheel jump, which hardcoding never could.
- **Resolves all keys** bound to an action (LCtrl *and* C both register as crouch, automatically).
- **`Input` mirrors the keybind menu** (`Jump, Crouch, ToggleCrouch, Fire, Aim, Reload, Sprint, MoveForward, …`); each maps to its real command verb(s), taken verbatim from TF2's `kb_act.lst` / `config_default_pc.cfg`.

## Safety / scope

- **Windows-only** — it reads `engine.dll` memory. Not host-testable.
- Every engine dereference is `VirtualQuery`-guarded (a Rust port of FzzyMod's `IsMemoryReadable`), so a wrong bind-table offset on a future game build resolves nothing rather than crashing.
- `refresh()` / `is_down()` / `matches()` should be driven from the engine thread (runframe) and the input hook, respectively. TF2 is frozen (no patches), so the bind-table offset is stable; it's still guarded for safety.

## Credit

Bind-table layout and the `IsMemoryReadable` approach are from the TF2 speedrun community's [FzzyMod](https://github.com/Fzzy2j/FzzyMod) (Fzzy2j) / TF2SR tooling.
