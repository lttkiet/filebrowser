#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    format_modified, has_graphical_session, read_file_clipboard, write_file_clipboard,
};

#[cfg(windows)]
pub fn locations() -> Vec<std::path::PathBuf> {
    ('A'..='Z')
        .map(|drive| std::path::PathBuf::from(format!("{drive}:\\")))
        .filter(|path| path.is_dir())
        .collect()
}

#[cfg(target_os = "macos")]
pub fn has_graphical_session() -> bool {
    std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
        || (std::env::var_os("SSH_TTY").is_none()
            && std::env::var_os("SSH_CONNECTION").is_none())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn has_graphical_session() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "macos")]
pub fn locations() -> Vec<std::path::PathBuf> {
    let mut paths = vec![std::path::PathBuf::from("/")];
    if let Ok(volumes) = std::fs::read_dir("/Volumes") {
        paths.extend(
            volumes
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_dir() && same_volume(path, std::path::Path::new("/")) != Some(true)
                }),
        );
    }
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(target_os = "linux")]
pub fn locations() -> Vec<std::path::PathBuf> {
    let mut paths = vec![std::path::PathBuf::from("/")];
    const PSEUDO_FILESYSTEMS: &[&str] = &[
        "autofs",
        "binfmt_misc",
        "bpf",
        "cgroup",
        "cgroup2",
        "configfs",
        "debugfs",
        "devpts",
        "devtmpfs",
        "efivarfs",
        "fusectl",
        "hugetlbfs",
        "mqueue",
        "nsfs",
        "overlay",
        "proc",
        "pstore",
        "ramfs",
        "rpc_pipefs",
        "securityfs",
        "squashfs",
        "sysfs",
        "tmpfs",
        "tracefs",
    ];
    if let Ok(mounts) = std::fs::read_to_string("/proc/self/mountinfo") {
        for line in mounts.lines() {
            let Some((mount, filesystem)) = line.split_once(" - ") else {
                continue;
            };
            let Some(mountpoint) = mount.split_whitespace().nth(4) else {
                continue;
            };
            let mut filesystem_fields = filesystem.split_whitespace();
            let Some(filesystem) = filesystem_fields.next() else {
                continue;
            };
            if PSEUDO_FILESYSTEMS.contains(&filesystem) {
                continue;
            }
            let Some(source) = filesystem_fields.next() else {
                continue;
            };
            let device = source.starts_with("/dev/") && !source.starts_with("/dev/loop");
            let network = matches!(
                filesystem,
                "9p" | "afs" | "ceph" | "cifs" | "davfs" | "glusterfs" | "nfs" | "nfs4"
                    | "smb3" | "virtiofs" | "zfs"
            );
            let fuse = filesystem == "fuse"
                || filesystem == "fuseblk"
                || (filesystem.starts_with("fuse.") && filesystem != "fuse.portal");
            if !(device || network || fuse) {
                continue;
            }
            let path = std::path::PathBuf::from(decode_mount_field(mountpoint));
            let duplicate_root_volume = device
                && same_volume(&path, std::path::Path::new("/")) == Some(true);
            if path.is_dir() && !duplicate_root_volume {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn locations() -> Vec<std::path::PathBuf> {
    vec![std::path::PathBuf::from("/")]
}

#[cfg(target_os = "linux")]
fn decode_mount_field(field: &str) -> String {
    let mut bytes = Vec::with_capacity(field.len());
    let mut field = field.as_bytes().iter().copied();
    while let Some(byte) = field.next() {
        if byte == b'\\' {
            let digits = [field.next(), field.next(), field.next()];
            if let [Some(a), Some(b), Some(c)] = digits
                && [a, b, c]
                    .iter()
                    .all(|digit| (b'0'..=b'7').contains(digit))
                && let Some(decoded) = (a - b'0')
                    .checked_mul(64)
                    .and_then(|value| value.checked_add((b - b'0') * 8))
                    .and_then(|value| value.checked_add(c - b'0'))
            {
                bytes.push(decoded);
                continue;
            }
            bytes.push(byte);
            bytes.extend(digits.into_iter().flatten());
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn trash_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Recycle Bin"
    } else {
        "Trash"
    }
}

#[cfg(not(windows))]
pub fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    format_modified_local(modified)
}

pub(crate) fn format_modified_local(modified: Option<std::time::SystemTime>) -> String {
    use chrono::{Datelike, Local, Timelike};

    let Some(modified) = modified else {
        return String::new();
    };
    let local = chrono::DateTime::<Local>::from(modified);
    let hour = match local.hour() % 12 {
        0 => 12,
        hour => hour,
    };
    let suffix = if local.hour() < 12 { "AM" } else { "PM" };
    format!(
        "{}/{}/{} {}:{:02} {}",
        local.month(),
        local.day(),
        local.year(),
        hour,
        local.minute(),
        suffix
    )
}

#[cfg(not(windows))]
pub fn read_file_clipboard() -> anyhow::Result<(Vec<std::path::PathBuf>, bool)> {
    anyhow::bail!("File clipboard integration is not available on this platform yet")
}

#[cfg(not(windows))]
pub fn write_file_clipboard(_: &[std::path::PathBuf], _: bool) -> anyhow::Result<()> {
    anyhow::bail!("File clipboard integration is not available on this platform yet")
}

#[cfg(windows)]
pub fn same_volume(source: &std::path::Path, destination: &std::path::Path) -> Option<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };

    let volume_name = |path: &std::path::Path| {
        let path = path.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
        let mut volume_path = vec![0_u16; 32_768];
        if unsafe {
            GetVolumePathNameW(
                path.as_ptr(),
                volume_path.as_mut_ptr(),
                volume_path.len() as u32,
            )
        } == 0
        {
            return None;
        }
        let mut volume_name = vec![0_u16; 32_768];
        if unsafe {
            GetVolumeNameForVolumeMountPointW(
                volume_path.as_ptr(),
                volume_name.as_mut_ptr(),
                volume_name.len() as u32,
            )
        } == 0
        {
            return None;
        }
        let length = volume_name.iter().position(|unit| *unit == 0)?;
        Some(volume_name[..length].to_vec())
    };

    Some(volume_name(source)? == volume_name(destination)?)
}

#[cfg(unix)]
pub fn same_volume(source: &std::path::Path, destination: &std::path::Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;

    let source = std::fs::symlink_metadata(source).ok()?.dev();
    let destination = std::fs::metadata(destination).ok()?.dev();
    Some(source == destination)
}

#[cfg(not(any(windows, unix)))]
pub fn same_volume(_: &std::path::Path, _: &std::path::Path) -> Option<bool> {
    None
}
