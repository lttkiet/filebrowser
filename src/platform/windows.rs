use std::{
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr::{copy_nonoverlapping, null_mut},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::bail;
use windows_sys::Win32::{
    Foundation::{FILETIME, GlobalFree, SYSTEMTIME},
    Storage::FileSystem::FileTimeToLocalFileTime,
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        Time::FileTimeToSystemTime,
    },
    UI::Shell::DragQueryFileW,
};

const CF_HDROP: u32 = 15;
const DROP_EFFECT_COPY: u32 = 1;
const DROP_EFFECT_MOVE: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct DropFiles {
    offset: u32,
    x: i32,
    y: i32,
    non_client: i32,
    wide: i32,
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> anyhow::Result<Self> {
        if unsafe { OpenClipboard(null_mut()) } == 0 {
            bail!("Could not open the Windows clipboard");
        }
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

pub fn has_graphical_session() -> bool {
    true
}

pub fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
        return super::format_modified_local(Some(modified));
    };
    let ticks = (duration.as_secs() + 11_644_473_600) * 10_000_000
        + u64::from(duration.subsec_nanos() / 100);
    let utc = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local = unsafe { std::mem::zeroed::<FILETIME>() };
    let mut date = unsafe { std::mem::zeroed::<SYSTEMTIME>() };
    if unsafe { FileTimeToLocalFileTime(&utc, &mut local) } != 0
        && unsafe { FileTimeToSystemTime(&local, &mut date) } != 0
    {
        let hour = match date.wHour % 12 {
            0 => 12,
            hour => hour,
        };
        let suffix = if date.wHour < 12 { "AM" } else { "PM" };
        format!(
            "{}/{}/{}  {}:{:02} {}",
            date.wMonth, date.wDay, date.wYear, hour, date.wMinute, suffix
        )
    } else {
        super::format_modified_local(Some(modified))
    }
}

pub fn write_file_clipboard(paths: &[PathBuf], cut: bool) -> anyhow::Result<()> {
    if paths.is_empty() {
        bail!("Select at least one item first");
    }
    let _guard = ClipboardGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        bail!("Could not clear the Windows clipboard");
    }

    let mut wide_paths = Vec::<u16>::new();
    for path in paths {
        wide_paths.extend(path.as_os_str().encode_wide());
        wide_paths.push(0);
    }
    wide_paths.push(0);

    let header = DropFiles {
        offset: size_of::<DropFiles>() as u32,
        x: 0,
        y: 0,
        non_client: 0,
        wide: 1,
    };
    let total_size = size_of::<DropFiles>() + wide_paths.len() * size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, total_size) };
    if memory.is_null() {
        bail!("Could not allocate clipboard data");
    }
    let destination = unsafe { GlobalLock(memory) };
    if destination.is_null() {
        unsafe { GlobalFree(memory) };
        bail!("Could not lock clipboard data");
    }
    unsafe {
        (destination as *mut DropFiles).write(header);
        copy_nonoverlapping(
            wide_paths.as_ptr(),
            (destination as *mut u8).add(size_of::<DropFiles>()) as *mut u16,
            wide_paths.len(),
        );
        GlobalUnlock(memory);
    }
    if unsafe { SetClipboardData(CF_HDROP, memory) }.is_null() {
        unsafe { GlobalFree(memory) };
        bail!("Could not publish file paths to the Windows clipboard");
    }

    let format = register_drop_effect_format();
    let effect = if cut {
        DROP_EFFECT_MOVE
    } else {
        DROP_EFFECT_COPY
    };
    let effect_memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, size_of::<u32>()) };
    if effect_memory.is_null() {
        return Ok(());
    }
    let effect_data = unsafe { GlobalLock(effect_memory) };
    if effect_data.is_null() {
        unsafe { GlobalFree(effect_memory) };
        return Ok(());
    }
    unsafe {
        (effect_data as *mut u32).write(effect);
        GlobalUnlock(effect_memory);
    }
    if unsafe { SetClipboardData(format, effect_memory) }.is_null() {
        unsafe { GlobalFree(effect_memory) };
    }
    Ok(())
}

pub fn read_file_clipboard() -> anyhow::Result<(Vec<PathBuf>, bool)> {
    let _guard = ClipboardGuard::open()?;
    let handle = unsafe { GetClipboardData(CF_HDROP) };
    if handle.is_null() {
        bail!("The clipboard does not contain file paths");
    }
    let count = unsafe { DragQueryFileW(handle, u32::MAX, null_mut(), 0) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = unsafe { DragQueryFileW(handle, index, null_mut(), 0) };
        let mut wide = vec![0u16; length as usize + 1];
        unsafe { DragQueryFileW(handle, index, wide.as_mut_ptr(), wide.len() as u32) };
        wide.truncate(length as usize);
        paths.push(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }

    let mut is_cut = false;
    let effect_handle = unsafe { GetClipboardData(register_drop_effect_format()) };
    if !effect_handle.is_null() {
        let effect_data = unsafe { GlobalLock(effect_handle) };
        if !effect_data.is_null() {
            is_cut = unsafe { (effect_data as *const u32).read() == DROP_EFFECT_MOVE };
            unsafe { GlobalUnlock(effect_handle) };
        }
    }
    Ok((paths, is_cut))
}

fn register_drop_effect_format() -> u32 {
    let name = "Preferred DropEffect\0".encode_utf16().collect::<Vec<_>>();
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}
