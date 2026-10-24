//! The disks: how much free space the disk that holds a folder has.

use std::path::Path;

/// The free space an unpack leaves, beside the unpacked size.
const SPARE: u64 = 64 * 1024 * 1024;

/// Whether `total` bytes fit in `free` with [`SPARE`] left; a disk that doesn't say has room.
pub fn room(total: u64, free: Option<u64>) -> Result<(), String> {
    match free {
        Some(free) if free < total.saturating_add(SPARE) => Err(format!(
            "This archive needs {} free, and the disk has {}",
            gigabytes(total.saturating_add(SPARE)),
            gigabytes(free)
        )),
        _ => Ok(()),
    }
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// The free bytes on the disk that holds `path`; `None` when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a struct this function owns.
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid C string and `stat` a writable statvfs.
        (unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` ends with a nul and `free` is a writable u64.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_bigger_than_the_free_space_is_refused() {
        assert!(room(10, Some(SPARE + 10)).is_ok());
        let refused = room(5_000_000_000, Some(1_000_000_000)).unwrap_err();
        assert!(refused.contains("needs 5.1 GB free, and the disk has 1.0 GB"), "{refused}");
        assert!(room(u64::MAX, None).is_ok(), "a disk that doesn't say");
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|f| f > 0));
    }
}
