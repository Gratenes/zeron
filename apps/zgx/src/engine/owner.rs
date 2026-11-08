//! Which program listens on the engine's port: to say what holds it when it
//! refuses this app's key, and to remember which desktop app's engine a
//! development build used (src/engine/manager.rs, `remember_engine_app`).

use std::path::PathBuf;

/// The executable of the process listening on TCP `port` (IPv4), if it can be
/// told.
#[cfg(windows)]
pub fn listener_exe(port: u16) -> Option<PathBuf> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    let mut size = 0u32;
    // SAFETY: a null table with a zero size only asks for the size it needs.
    unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0) };
    if size == 0 {
        return None;
    }
    // (u32s: the table's alignment; room for a few rows more meanwhile.)
    let mut buffer = vec![0u32; (size as usize).div_ceil(4) + 256];
    let mut size = (buffer.len() * 4) as u32;
    // SAFETY: `buffer` is `size` bytes, as the call is told.
    let status = unsafe {
        GetExtendedTcpTable(buffer.as_mut_ptr().cast(), &mut size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
    };
    if status != 0 {
        return None;
    }
    let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
    // SAFETY: the call filled `buffer` with a table of `dwNumEntries` rows.
    let rows = unsafe { std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize) };
    let pid = rows.iter().find(|row| u16::from_be((row.dwLocalPort & 0xffff) as u16) == port)?.dwOwningPid;
    process_exe(pid)
}

#[cfg(windows)]
fn process_exe(pid: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    // SAFETY: plain calls; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut name = vec![0u16; 32_768];
    let mut len = name.len() as u32;
    // SAFETY: `name` holds `len` UTF-16 units.
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, name.as_mut_ptr(), &mut len) };
    // SAFETY: opened above.
    unsafe { CloseHandle(process) };
    (ok != 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&name[..len as usize])))
}

#[cfg(not(windows))]
pub fn listener_exe(_port: u16) -> Option<PathBuf> {
    None
}
