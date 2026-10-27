//! MO2 stays alive while the game runs (it hosts the virtual file system) and shows a "locked" window with an
//! Unlock button over that time. The launcher has its own "game is running" screen, so the window is hidden.

use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::commands::{GAME_PROCESS, MO2_PROCESS};

/// The game may take a while to show up (REDmod deploy, Steam); past this the launch is considered failed.
const GAME_WAIT: Duration = Duration::from_secs(180);
const TICK: Duration = Duration::from_millis(250);

/// Hides MO2's windows until the game it started is over. Only for an MO2 the launch itself brought up: a window
/// the player opened on their own stays.
pub fn hide_until_game_over() {
    std::thread::spawn(|| {
        let mut sys = System::new();
        let (start, mut game_seen) = (Instant::now(), false);
        loop {
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
            let (mut game, mut mo2) = (false, Vec::new());
            for (pid, p) in sys.processes() {
                let name = p.name().to_string_lossy();
                game |= name.eq_ignore_ascii_case(GAME_PROCESS);
                if name.eq_ignore_ascii_case(MO2_PROCESS) {
                    mo2.push(pid.as_u32());
                }
            }
            game_seen |= game;
            if (game_seen && !game) || (mo2.is_empty() && game_seen) || start.elapsed() > GAME_WAIT {
                return;
            }
            hide_windows(&mo2);
            std::thread::sleep(TICK);
        }
    });
}

#[cfg(windows)]
fn hide_windows(pids: &[u32]) {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, ShowWindow, SW_HIDE,
    };

    unsafe extern "system" fn each(hwnd: HWND, pids: LPARAM) -> BOOL {
        let pids = &*(pids as *const &[u32]);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pids.contains(&pid) && IsWindowVisible(hwnd) != 0 {
            ShowWindow(hwnd, SW_HIDE);
        }
        1
    }
    if !pids.is_empty() {
        unsafe { EnumWindows(Some(each), &pids as *const &[u32] as LPARAM) };
    }
}

// No MO2 off Windows (dev only).
#[cfg(not(windows))]
fn hide_windows(_: &[u32]) {}
