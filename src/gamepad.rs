/// Controller input helper — reads the assigned gamepad via gilrs (XInput backend
/// only; WGI disabled) and injects keyboard events directly into the launcher
/// window via PostMessage, targeting the specific HWND owned by the process we
/// spawned — NOT gated on OS-level window focus.
///
/// This matters specifically for PartyDeck (Wine + gamescope split-screen):
/// PartyDeck isolates controllers per-instance purely via evdev masking, not by
/// giving each instance's window OS focus — with several instances tiled and
/// visible at once, only one window ever has true focus at a time. Two things
/// depend on that focus and are both wrong here:
///   1. `SendInput` (the previous approach) is focus-gated even under Wine —
///      wineserver only routes injected hardware input to the focused window.
///   2. Ultralight/AppCore's OverlayManager only accepts key events once its
///      View has been internally focused, which normally only happens via a
///      real WM_SETFOCUS — which a non-focused window never receives.
/// `PostMessage` posts directly onto a specific window's message queue
/// regardless of focus, sidestepping (1); explicitly posting WM_SETFOCUS
/// primes Ultralight's internal focus state, sidestepping (2).
///
/// Targeting by the PID we ourselves spawned (rather than guessing via window
/// class name alone, which is ambiguous once more than one instance is
/// running) also fixes a real correctness bug: the previous approach could
/// inject into a *different* PartyDeck instance's identically-classed window.
///
/// Run this on a background thread after launching the launcher.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::thread;

use anyhow::Result;
use gilrs::{Axis, Button, Event, EventType, Gilrs};

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, MAPVK_VK_TO_VSC,
    VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_RETURN, VK_ESCAPE, VK_TAB, VK_LSHIFT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    PostMessageW, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SETFOCUS,
};
#[cfg(windows)]
use windows::Win32::System::SystemServices::MK_LBUTTON;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM, BOOL};

/// Window class name for the Plutonium launcher (Ultralight native shell).
const LAUNCHER_WINDOW_CLASS: &str = "UltralightWindow";

/// Axis dead-zone (0.0–1.0 scale; gilrs reports -1.0..1.0).
const DEAD_ZONE: f32 = 0.5;
/// Minimum milliseconds between repeated key injections (auto-repeat rate).
const REPEAT_MS: u64 = 150;
/// How often to re-post WM_SETFOCUS so Ultralight's Overlay keeps accepting
/// input even if real OS focus moves to another window/instance.
const FOCUS_PRIME_MS: u64 = 1000;
/// How often to retry finding the target window if it hasn't appeared yet.
const FIND_WINDOW_RETRY_MS: u64 = 200;

/// Spawn the controller helper on a background thread.
/// The thread runs until `stop` is set to true.
pub fn spawn_controller_helper(ui_pid: u32, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_controller_loop(ui_pid, stop) {
            eprintln!("[gamepad] controller helper error: {}", e);
        }
    })
}

fn run_controller_loop(ui_pid: u32, stop: Arc<AtomicBool>) -> Result<()> {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[gamepad] gilrs init failed: {}. Controller navigation disabled.", e);
            return Ok(());
        }
    };

    // Report detected gamepads at startup.
    let pad_count = gilrs.gamepads().count();
    if pad_count == 0 {
        eprintln!("[gamepad] No gamepads detected via XInput. Controller navigation disabled.");
        eprintln!("[gamepad] (Under Proton/PartyDeck: check winebus/evdev masking.)");
        return Ok(());
    }
    for (id, pad) in gilrs.gamepads() {
        println!("[gamepad] Found: {:?} {:?}", id, pad.name());
    }

    println!("[gamepad] Controller helper active. Targeting launcher window by pid {}.", ui_pid);

    let mut last_inject = Instant::now();
    let mut last_focus_prime = Instant::now() - Duration::from_millis(FOCUS_PRIME_MS);
    let mut target_hwnd: Option<HWND> = None;

    while !stop.load(Ordering::Relaxed) {
        // Drain gilrs events to keep state up to date.
        while let Some(Event { event, .. }) = gilrs.next_event() {
            match event {
                EventType::Connected => println!("[gamepad] Gamepad connected"),
                EventType::Disconnected => println!("[gamepad] Gamepad disconnected"),
                _ => {}
            }
        }

        // Resolve (or revalidate) the target window. No focus check — we
        // inject directly into this specific window's queue regardless of
        // which window currently has OS focus.
        if !hwnd_is_valid(target_hwnd) {
            target_hwnd = find_launcher_window(ui_pid);
            if let Some(h) = target_hwnd {
                println!("[gamepad] Found launcher window for pid {} ({:?})", ui_pid, h);
                // Full prime (focus + synthetic click) once per resolution —
                // establishes Ultralight's focused_overlay_ the first time.
                prime_focus_full(h);
                last_focus_prime = Instant::now();
            }
        }

        let Some(hwnd) = target_hwnd else {
            thread::sleep(Duration::from_millis(FIND_WINDOW_RETRY_MS));
            continue;
        };

        // Periodically re-assert focus at the Ultralight/AppCore level so its
        // Overlay keeps accepting input even without real OS focus. Light
        // variant only — no repeated synthetic clicks once already primed.
        if last_focus_prime.elapsed() >= Duration::from_millis(FOCUS_PRIME_MS) {
            prime_focus_light(hwnd);
            last_focus_prime = Instant::now();
        }

        // Throttle repeat rate.
        if last_inject.elapsed() < Duration::from_millis(REPEAT_MS) {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Check button + axis state across all connected pads.
        for (_, pad) in gilrs.gamepads() {
            // Directional: D-pad buttons or left stick.
            let up = pad.is_pressed(Button::DPadUp)
                || pad.axis_data(Axis::LeftStickY).map_or(false, |a| a.value() > DEAD_ZONE);
            let down = pad.is_pressed(Button::DPadDown)
                || pad.axis_data(Axis::LeftStickY).map_or(false, |a| a.value() < -DEAD_ZONE);
            let left = pad.is_pressed(Button::DPadLeft)
                || pad.axis_data(Axis::LeftStickX).map_or(false, |a| a.value() < -DEAD_ZONE);
            let right = pad.is_pressed(Button::DPadRight)
                || pad.axis_data(Axis::LeftStickX).map_or(false, |a| a.value() > DEAD_ZONE);

            let confirm = pad.is_pressed(Button::South);   // A / Cross
            let back    = pad.is_pressed(Button::East);    // B / Circle
            let tab_fwd = pad.is_pressed(Button::RightTrigger);  // RB / R1 — tab forward
            let tab_bck = pad.is_pressed(Button::LeftTrigger);   // LB / L1 — tab backward

            // Map to virtual keys.
            let vk = if up        { Some(VK_UP) }
                else if down      { Some(VK_DOWN) }
                else if left      { Some(VK_LEFT) }
                else if right     { Some(VK_RIGHT) }
                else if confirm   { Some(VK_RETURN) }
                else if back      { Some(VK_ESCAPE) }
                else if tab_fwd   { Some(VK_TAB) }
                else if tab_bck   { Some(VK_TAB) }  // shift+tab handled separately below
                else              { None };

            if let Some(key) = vk {
                inject_key(hwnd, key.0, tab_bck /* shift modifier for shift+tab */);
                last_inject = Instant::now();
                break; // one key per cycle per pad
            }
        }

        thread::sleep(Duration::from_millis(10));
    }

    println!("[gamepad] Controller helper stopped.");
    Ok(())
}

/// True if `hwnd` is Some and still a valid window (the launcher may have
/// closed/relaunched, in which case we need to re-resolve it).
#[cfg(windows)]
fn hwnd_is_valid(hwnd: Option<HWND>) -> bool {
    match hwnd {
        Some(h) => unsafe { IsWindow(h).as_bool() },
        None => false,
    }
}
#[cfg(not(windows))]
fn hwnd_is_valid(hwnd: Option<HWND>) -> bool {
    hwnd.is_some()
}

/// Find the top-level `UltralightWindow` owned by `target_pid`.
///
/// We match on the exact PID we spawned rather than walking descendants: our
/// `launch_target` (from prod.json) is Plutonium's actual UI binary directly,
/// not a bootstrapper that re-execs further — confirmed empirically (the
/// window's owning PID matches the process we spawn). If that ever changes,
/// this is the place to add a descendant-PID walk (via Toolhelp, as
/// `main.rs::process_is_running` already does) before matching by class.
#[cfg(windows)]
fn find_launcher_window(target_pid: u32) -> Option<HWND> {
    struct FindContext {
        target_pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let ctx = &mut *(lparam.0 as *mut FindContext);

            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != ctx.target_pid {
                return BOOL(1); // continue enumeration
            }

            let mut class_buf = [0u16; 256];
            let len = GetClassNameW(hwnd, &mut class_buf);
            if len == 0 {
                return BOOL(1);
            }
            let class = String::from_utf16_lossy(&class_buf[..len as usize]);
            if class != LAUNCHER_WINDOW_CLASS {
                return BOOL(1);
            }

            if IsWindowVisible(hwnd).as_bool() {
                ctx.found = Some(hwnd);
                return BOOL(0); // stop enumeration
            }

            BOOL(1)
        }
    }

    let mut ctx = FindContext { target_pid, found: None };
    unsafe {
        let ctx_ptr: *mut FindContext = &mut ctx;
        let _ = EnumWindows(Some(callback), LPARAM(ctx_ptr as isize));
    }
    ctx.found
}
#[cfg(not(windows))]
fn find_launcher_window(_target_pid: u32) -> Option<HWND> {
    None
}

/// Post WM_SETFOCUS to prime Ultralight/AppCore's internal Overlay focus
/// state, independent of real OS-level window focus. Cheap; safe to call
/// repeatedly (e.g. to re-assert focus periodically).
#[cfg(windows)]
fn prime_focus_light(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(hwnd, WM_SETFOCUS, WPARAM(0), LPARAM(0));
    }
}
#[cfg(not(windows))]
fn prime_focus_light(_hwnd: HWND) {}

/// Full priming: WM_SETFOCUS plus a synthetic click.
///
/// WM_SETFOCUS alone only flips AppCore's `window_focused_` flag; per
/// OverlayManager's source, it calls `view()->Focus()` only on whatever
/// overlay is *already* `focused_overlay_` — which itself is only set by a
/// real mouse-click hit-test (`FireMouseEvent`), never by focus alone. If the
/// overlay was never clicked, WM_SETFOCUS has nothing to act on. A synthetic
/// click forces `focused_overlay_` to be set. Call this once per window
/// resolution, not on every periodic re-prime (posting real clicks
/// repeatedly risks hitting a real UI element if the page has navigated).
#[cfg(windows)]
fn prime_focus_full(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(hwnd, WM_SETFOCUS, WPARAM(0), LPARAM(0));
        // Harmless top-left corner, unlikely to land on a real control.
        let point_lparam = LPARAM((1i32 | (1i32 << 16)) as isize);
        let _ = PostMessageW(hwnd, WM_LBUTTONDOWN, WPARAM(MK_LBUTTON.0 as usize), point_lparam);
        let _ = PostMessageW(hwnd, WM_LBUTTONUP, WPARAM(0), point_lparam);
    }
}
#[cfg(not(windows))]
fn prime_focus_full(_hwnd: HWND) {}

/// Build the lParam Windows would generate for a real key event, so the
/// resulting KeyEvent (and therefore the JS KeyboardEvent) matches what a
/// physical keypress produces. Ultralight needs the scan code to populate
/// modern KeyboardEvent fields correctly.
#[cfg(windows)]
fn key_lparam(vk: u16, key_up: bool) -> u32 {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } & 0xFF;
    let extended: u32 =
        if vk == VK_UP.0 || vk == VK_DOWN.0 || vk == VK_LEFT.0 || vk == VK_RIGHT.0 {
            0x0100_0000
        } else {
            0
        };
    let scan_field = scan << 16;
    if key_up {
        0xC000_0001 | scan_field | extended
    } else {
        0x0000_0001 | scan_field | extended
    }
}

/// Post a single key press+release directly to `hwnd`'s message queue.
/// If `shift` is true, also post Left Shift around it (for Shift+Tab).
#[cfg(windows)]
fn inject_key(hwnd: HWND, vk: u16, shift: bool) {
    unsafe {
        if shift {
            post_key(hwnd, VK_LSHIFT.0, false);
        }
        post_key(hwnd, vk, false);
        post_key(hwnd, vk, true);
        if shift {
            post_key(hwnd, VK_LSHIFT.0, true);
        }
    }
}

#[cfg(windows)]
unsafe fn post_key(hwnd: HWND, vk: u16, key_up: bool) {
    let msg = if key_up { WM_KEYUP } else { WM_KEYDOWN };
    let lparam = key_lparam(vk, key_up);
    unsafe {
        let _ = PostMessageW(hwnd, msg, WPARAM(vk as usize), LPARAM(lparam as isize));
    }
}

#[cfg(not(windows))]
fn inject_key(_hwnd: HWND, _vk: u16, _shift: bool) {
    // No-op on non-Windows (gilrs still compiles but PostMessage doesn't exist).
}
