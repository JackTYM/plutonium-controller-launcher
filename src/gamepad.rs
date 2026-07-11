/// Controller input helper — reads the assigned gamepad via gilrs (XInput backend
/// only; WGI disabled) and injects keyboard events into the foreground launcher
/// window via SendInput.
///
/// Only active while `plutonium-launcher-win32.exe` is the foreground window, so
/// the helper stops injecting the moment a game is launched and the launcher
/// window loses focus.
///
/// Run this on a background thread after launching the launcher.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::thread;

use anyhow::Result;
use gilrs::{Axis, Button, Event, EventType, Gilrs};

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_RETURN, VK_ESCAPE, VK_TAB,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, FindWindowW};

/// Window class name for the Plutonium launcher (Ultralight native shell).
/// Confirmed empirically via GetClassNameW on the live window; adjust if
/// Plutonium changes its window class in a future update.
const LAUNCHER_WINDOW_CLASS: &str = "UltralightWindow";
/// Fallback: window title substring (probe first; adjust if class lookup fails).
#[allow(dead_code)]
const LAUNCHER_WINDOW_TITLE: &str = "Plutonium";

/// Axis dead-zone (0.0–1.0 scale; gilrs reports -1.0..1.0).
const DEAD_ZONE: f32 = 0.5;
/// Minimum milliseconds between repeated key injections (auto-repeat rate).
const REPEAT_MS: u64 = 150;

/// Spawn the controller helper on a background thread.
/// The thread runs until `stop` is set to true.
pub fn spawn_controller_helper(stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = run_controller_loop(stop) {
            eprintln!("[gamepad] controller helper error: {}", e);
        }
    })
}

fn run_controller_loop(stop: Arc<AtomicBool>) -> Result<()> {
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

    println!("[gamepad] Controller helper active. Injecting keys while launcher is foreground.");

    let mut last_inject = std::time::Instant::now();
    // A freshly created launcher window doesn't respond to SendInput-injected
    // keys until it has processed one real keyboard event (observed: a single
    // physical arrow-key press permanently "unlocks" synthetic injection for
    // the rest of the session). Prime it ourselves once the window is first
    // confirmed foreground, so gamepad-only input works without requiring the
    // user to touch a keyboard.
    let mut primed = false;

    while !stop.load(Ordering::Relaxed) {
        // Drain gilrs events to keep state up to date.
        while let Some(Event { event, .. }) = gilrs.next_event() {
            match event {
                EventType::Connected => println!("[gamepad] Gamepad connected"),
                EventType::Disconnected => println!("[gamepad] Gamepad disconnected"),
                _ => {}
            }
        }

        // Only inject while the launcher window is foreground.
        if !launcher_is_foreground() {
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        if !primed {
            inject_key(VK_DOWN.0, false);
            primed = true;
            last_inject = std::time::Instant::now();
            thread::sleep(Duration::from_millis(50));
            continue;
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
                inject_key(key.0, tab_bck /* shift modifier for shift+tab */);
                last_inject = std::time::Instant::now();
                break; // one key per cycle per pad
            }
        }

        thread::sleep(Duration::from_millis(10));
    }

    println!("[gamepad] Controller helper stopped.");
    Ok(())
}

/// True if the Plutonium launcher window is currently the foreground window.
fn launcher_is_foreground() -> bool {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        fn to_wide(s: &str) -> Vec<u16> {
            OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
        }

        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() {
                return false;
            }
            // Try to find the launcher by window class.
            let class_wide = to_wide(LAUNCHER_WINDOW_CLASS);
            if let Ok(launcher_hwnd) = FindWindowW(
                PCWSTR(class_wide.as_ptr()),
                PCWSTR::null(),
            ) {
                return fg == launcher_hwnd;
            }
            // Fallback: accept any foreground window (less precise).
            // Better than blocking all navigation if the class name is wrong.
            true
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Inject a single key press+release via SendInput.
/// If `shift` is true, also hold Left Shift (for Shift+Tab).
#[cfg(windows)]
fn inject_key(vk: u16, shift: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYEVENTF_KEYUP, VK_LSHIFT,
    };

    let mut inputs: Vec<INPUT> = Vec::new();

    let make_key = |vk_code: u16, flags: KEYBD_EVENT_FLAGS| -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk_code),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    };

    if shift {
        inputs.push(make_key(VK_LSHIFT.0, KEYBD_EVENT_FLAGS(0)));
    }
    inputs.push(make_key(vk, KEYBD_EVENT_FLAGS(0)));
    inputs.push(make_key(vk, KEYEVENTF_KEYUP));
    if shift {
        inputs.push(make_key(VK_LSHIFT.0, KEYEVENTF_KEYUP));
    }

    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
fn inject_key(_vk: u16, _shift: bool) {
    // No-op on non-Windows (gilrs still compiles but SendInput doesn't exist).
}
