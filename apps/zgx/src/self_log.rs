//! A build to double-click (src/baked.rs) has no launcher to catch its output
//! and, on Windows, no console: started that way, with no standard error to
//! write to, its stdout and stderr go to its engine's `logs\zgx.log`
//! (appended, a header line per launch). Rust's standard streams look the
//! process's handles up on every write, so setting them here, first thing,
//! takes for everything after. Started with streams of its own (a launcher
//! redirecting them, a terminal), it keeps those.

/// Sends stdout and stderr to the log, when this build bakes an engine data
/// dir and the process has no standard error of its own.
#[cfg(windows)]
pub fn when_detached() {
    use std::io::Write;
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};

    if crate::baked::route().is_none() {
        return;
    }
    // SAFETY: a plain query.
    let current = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if !current.is_null() && current != INVALID_HANDLE_VALUE {
        return;
    }
    let Some(data) = crate::baked::env_or_baked("ZERON_ZGX_ENGINE_DATA") else { return };
    let dir = std::path::Path::new(&data).join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("zgx.log")) else {
        return;
    };
    let args: Vec<String> = std::env::args().collect();
    let _ = writeln!(file, "===== {} launch: {} (pid {})", utc_now(), args.join(" "), std::process::id());
    // The process keeps the handle for as long as it runs.
    let handle = file.into_raw_handle();
    // SAFETY: `handle` is an open file handle this process owns from now on.
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
    }
}

#[cfg(not(windows))]
pub fn when_detached() {}

/// Now, as `2026-11-07 21:32:39Z`.
#[cfg(windows)]
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}
