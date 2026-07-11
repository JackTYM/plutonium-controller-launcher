#![windows_subsystem = "windows"]

/// Plutonium controller launcher — entry point.
///
/// Modes:
///   (default)      Update all files, write patched index.html, launch, run controller helper.
///   --no-update    Skip update step (PartyDeck: run once to update, then N times with this flag).
///   --update-only  Update + patch but don't launch (for pre-seeding the install).
///   --install-dir <path>   Override the install directory (default: %LOCALAPPDATA%\Plutonium).
///   --full-verify  Full SHA1 check instead of size-only (slower, more thorough).
///                  Size-only is the default, matching stock Plutonium's fastVerify behavior.

mod gamepad;
mod manifest;
mod patch;
mod updater;

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use updater::Updater;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:?}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    let no_update = args.contains(&"--no-update".to_string());
    let update_only = args.contains(&"--update-only".to_string());
    let full_verify = args.contains(&"--full-verify".to_string());

    let install_dir = parse_install_dir(&args)?;
    println!("Install dir: {}", install_dir.display());

    let upd = Updater::new(install_dir).with_fast_verify(!full_verify);

    // PID of the spawned launcher process. Needed (not just cosmetic) so the
    // gamepad helper can target our specific window and so we know exactly
    // when *our* instance closes — both matter under PartyDeck, where several
    // simultaneous instances can share the same process name and window class.
    let ui_pid = if no_update {
        // Fast path: re-apply patch and launch; no network.
        println!("Skipping update (--no-update).");
        upd.launch_only()?
    } else if update_only {
        println!("Updating (--update-only, no launch).");
        upd.sync()?;
        println!("Update complete.");
        return Ok(());
    } else {
        // Default: update + patch + launch.
        upd.run()?
    };

    // Spawn the controller helper as a resident background thread.
    // It exits when `stop` is set, which we set when the launcher process is gone.
    let stop = Arc::new(AtomicBool::new(false));
    let helper_stop = stop.clone();
    let _helper = gamepad::spawn_controller_helper(ui_pid, helper_stop);

    // Keep this process alive until the launcher exits.
    wait_for_launcher(ui_pid);

    stop.store(true, Ordering::Relaxed);
    Ok(())
}

/// Block until the launcher process (by PID) is no longer running.
fn wait_for_launcher(ui_pid: u32) {
    use std::time::Duration;
    use std::thread;

    println!("Waiting for launcher to close…");
    loop {
        thread::sleep(Duration::from_secs(2));
        if !process_is_running(ui_pid) {
            println!("Launcher closed. Exiting.");
            break;
        }
    }
}

/// Returns true if a process with the given PID currently exists.
fn process_is_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW,
            PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        use windows::Win32::Foundation::CloseHandle;

        unsafe {
            let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return true, // assume running on error
            };

            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };

            let mut found = false;
            if Process32FirstW(snap, &mut entry).is_ok() {
                loop {
                    if entry.th32ProcessID == pid {
                        found = true;
                        break;
                    }
                    if Process32NextW(snap, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _: Result<(), _> = CloseHandle(snap);
            found
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

fn parse_install_dir(args: &[String]) -> Result<PathBuf> {
    // --install-dir=<path> or --install-dir <path>
    for (i, arg) in args.iter().enumerate() {
        if let Some(val) = arg.strip_prefix("--install-dir=") {
            return Ok(PathBuf::from(val));
        }
        if arg == "--install-dir" {
            if let Some(next) = args.get(i + 1) {
                return Ok(PathBuf::from(next));
            }
        }
    }

    // Default: %LOCALAPPDATA%\Plutonium  (mirrors the stock updater)
    let local_app_data = env::var("LOCALAPPDATA")
        .context("LOCALAPPDATA env var not set")?;
    Ok(PathBuf::from(local_app_data).join("Plutonium"))
}
