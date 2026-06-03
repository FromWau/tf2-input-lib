//! tf2-input — query Titanfall 2 input by **action**, not by hardcoded key.
//!
//! A player can bind an action to several keys (the keybind menu allows a primary + alternate,
//! and people add more). Hardcoding `KEY_SPACE`/`KEY_LCONTROL` breaks for anyone who rebinds.
//! Instead this reads the engine's key-bind table (the same one the game reads), resolves each
//! [`Input`] action to the set of `ButtonCode_t`s currently bound to it, and tracks which of
//! those are physically held — so a plugin can just ask:
//!
//! ```ignore
//! tf2_input::on_button_event(scan, pressed);   // from the PostEvent / input hook
//! tf2_input::refresh();                          // once engine.dll is loaded, + periodically
//! if tf2_input::is_down(Input::Jump) { ... }
//! ```
//!
//! Action → command verbs are taken verbatim from TF2's `kb_act.lst` / `config_default_pc.cfg`,
//! so they match what the keybind menu writes. The bind-table offset is from FzzyMod / TF2SR.
//!
//! Windows-only (reads engine.dll memory); guarded with `VirtualQuery` so a wrong offset on a
//! future game build is *safe* (resolves nothing) rather than a crash.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;
use winapi::um::libloaderapi::GetModuleHandleA;
use winapi::um::memoryapi::VirtualQuery;
use winapi::um::winnt::{MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_NOACCESS};

/// A bindable Titanfall 2 action (mirrors the in-game Keybind menu).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    // --- Actions ---
    Fire,
    Aim,
    ToggleAim,
    Reload,
    SwitchWeapons,
    SwitchToWeapon1,
    SwitchToWeapon2,
    SwitchToWeapon3,
    Melee,
    Ordnance,
    Tactical,
    TitanUtility,
    Use,
    EjectSafety,
    TitanMode,
    BurnCard,
    // --- Movement ---
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    Sprint,
    Jump,
    Crouch,
    ToggleCrouch,
    // --- Communication ---
    PushToTalk,
    Chat,
    TeamChat,
    Scoreboard,
}

/// Every [`Input`], in the SAME order as the enum's declaration (so `self as usize` indexes it).
pub const ALL: [Input; 28] = [
    Input::Fire,
    Input::Aim,
    Input::ToggleAim,
    Input::Reload,
    Input::SwitchWeapons,
    Input::SwitchToWeapon1,
    Input::SwitchToWeapon2,
    Input::SwitchToWeapon3,
    Input::Melee,
    Input::Ordnance,
    Input::Tactical,
    Input::TitanUtility,
    Input::Use,
    Input::EjectSafety,
    Input::TitanMode,
    Input::BurnCard,
    Input::MoveForward,
    Input::MoveBack,
    Input::MoveLeft,
    Input::MoveRight,
    Input::Sprint,
    Input::Jump,
    Input::Crouch,
    Input::ToggleCrouch,
    Input::PushToTalk,
    Input::Chat,
    Input::TeamChat,
    Input::Scoreboard,
];
const NUM_INPUTS: usize = ALL.len();

impl Input {
    #[inline]
    fn idx(self) -> usize {
        self as usize
    }

    /// Console command verb(s) that count as this action — verbatim from `kb_act.lst`.
    /// A bound key matches the action if its binding equals any of these.
    pub fn verbs(self) -> &'static [&'static str] {
        match self {
            Input::Fire => &["+attack"],
            Input::Aim => &["+zoom"],
            Input::ToggleAim => &["+toggle_zoom"],
            Input::Reload => &["+reload"],
            Input::SwitchWeapons => &["+weaponCycle"],
            Input::SwitchToWeapon1 => &["weaponSelectPrimary0"],
            Input::SwitchToWeapon2 => &["weaponSelectPrimary1"],
            Input::SwitchToWeapon3 => &["weaponSelectPrimary2"],
            Input::Melee => &["+melee"],
            Input::Ordnance => &["+offhand0"],
            Input::Tactical => &["+offhand1"],
            Input::TitanUtility => &["+offhand2"],
            Input::Use => &["+use"],
            Input::EjectSafety => &["+scriptCommand1"],
            Input::TitanMode => &["+ability 1"],
            Input::BurnCard => &["+ability 6"],
            Input::MoveForward => &["+forward"],
            Input::MoveBack => &["+back"],
            Input::MoveLeft => &["+moveleft"],
            Input::MoveRight => &["+moveright"],
            Input::Sprint => &["+speed"],
            Input::Jump => &["+ability 3", "+jump"],
            Input::Crouch => &["+duck"],
            Input::ToggleCrouch => &["+toggle_duck"],
            Input::PushToTalk => &["+pushtotalk"],
            Input::Chat => &["say"],
            Input::TeamChat => &["say_team"],
            Input::Scoreboard => &["+showscores"],
        }
    }
}

// engine.dll key-bind table: array indexed by ButtonCode_t, stride 0x10, with a pointer to the
// bound command string at offset 0 (from FzzyMod / TF2SR).
const BIND_TABLE_RVA: usize = 0x1396C5C0;
const BIND_STRIDE: usize = 0x10;
/// ButtonCode_t range we track (covers keyboard + mouse; controller codes are higher and ignored).
pub const BUTTON_CODE_COUNT: usize = 256;
const MAX_BINDS: usize = 4;

// Resolved ButtonCodes per action (0 = empty slot).
static BINDS: [[AtomicI32; MAX_BINDS]; NUM_INPUTS] =
    [const { [const { AtomicI32::new(0) }; MAX_BINDS] }; NUM_INPUTS];
// Physical key down-state, indexed by ButtonCode.
static KEY_DOWN: [AtomicBool; BUTTON_CODE_COUNT] = [const { AtomicBool::new(false) }; BUTTON_CODE_COUNT];

static ENGINE_BASE: OnceLock<usize> = OnceLock::new();

fn engine_base() -> Option<usize> {
    if let Some(b) = ENGINE_BASE.get() {
        return Some(*b);
    }
    let h = unsafe { GetModuleHandleA(c"engine.dll".as_ptr()) };
    if h.is_null() {
        return None;
    }
    let b = h as usize;
    let _ = ENGINE_BASE.set(b);
    Some(b)
}

/// Is `addr` in a committed, accessible page? (Rust port of FzzyMod's IsMemoryReadable.)
fn is_readable(addr: usize) -> bool {
    if addr == 0 {
        return false;
    }
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let n = unsafe {
        VirtualQuery(addr as *const _, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
    };
    n != 0 && (mbi.State & MEM_COMMIT) != 0 && (mbi.Protect & PAGE_NOACCESS) == 0
}

/// The command string bound to a ButtonCode, or None if unbound / unreadable (bounded, safe).
fn binding_for(engine: usize, code: usize) -> Option<String> {
    let slot = engine + BIND_TABLE_RVA + code * BIND_STRIDE;
    if !is_readable(slot) {
        return None;
    }
    let ptr = unsafe { *(slot as *const usize) };
    if !is_readable(ptr) {
        return None;
    }
    let mut bytes = Vec::new();
    for i in 0..63usize {
        if !is_readable(ptr + i) {
            break;
        }
        let b = unsafe { *((ptr + i) as *const u8) };
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn binding_matches(cmd: &str, input: Input) -> bool {
    let cmd = cmd.trim();
    input.verbs().iter().any(|v| cmd == *v)
}

/// Re-read the engine bind table and re-resolve every action's keys. Call once engine.dll is
/// loaded and periodically to pick up live rebinds. Returns true if the table was readable.
pub fn refresh() -> bool {
    let Some(engine) = engine_base() else { return false };
    let mut next = [[0i32; MAX_BINDS]; NUM_INPUTS];
    let mut count = [0usize; NUM_INPUTS];
    let mut found = false;
    for code in 0..BUTTON_CODE_COUNT {
        let Some(cmd) = binding_for(engine, code) else { continue };
        found = true;
        for input in ALL {
            let i = input.idx();
            if count[i] < MAX_BINDS && binding_matches(&cmd, input) {
                next[i][count[i]] = code as i32;
                count[i] += 1;
            }
        }
    }
    if !found {
        return false; // bindings not loaded yet — keep whatever we had
    }
    for i in 0..NUM_INPUTS {
        for s in 0..MAX_BINDS {
            BINDS[i][s].store(next[i][s], Ordering::Relaxed);
        }
    }
    true
}

/// Feed a raw button event — call from the input hook for every press/release.
pub fn on_button_event(scan: i32, pressed: bool) {
    if (0..BUTTON_CODE_COUNT as i32).contains(&scan) {
        KEY_DOWN[scan as usize].store(pressed, Ordering::Relaxed);
    }
}

/// Is `scan` one of the keys currently bound to `input`?
pub fn matches(input: Input, scan: i32) -> bool {
    if scan == 0 {
        return false;
    }
    BINDS[input.idx()].iter().any(|k| {
        let v = k.load(Ordering::Relaxed);
        v != 0 && v == scan
    })
}

/// Is any key bound to `input` currently held down?
pub fn is_down(input: Input) -> bool {
    BINDS[input.idx()].iter().any(|k| {
        let c = k.load(Ordering::Relaxed);
        c > 0 && (c as usize) < BUTTON_CODE_COUNT && KEY_DOWN[c as usize].load(Ordering::Relaxed)
    })
}

/// Resolved ButtonCodes for an action (trailing zeros = empty slots) — for logging/debug.
pub fn bound_keys(input: Input) -> [i32; MAX_BINDS] {
    let mut out = [0i32; MAX_BINDS];
    for s in 0..MAX_BINDS {
        out[s] = BINDS[input.idx()][s].load(Ordering::Relaxed);
    }
    out
}
