//! The disks: how much free space the disk that holds a folder has, and its size. And this
//! computer's name.

use std::path::Path;

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

/// The size in bytes of the disk that holds `path`; `None` when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a struct this function owns.
pub fn disk_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid C string and `stat` a writable statvfs.
        (unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0).then(|| stat.f_blocks as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut total = 0u64;
        // SAFETY: `wide` ends with a nul and `total` is a writable u64.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &mut total,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(total)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// This computer's name on the network; empty when the system doesn't say.
#[allow(unsafe_code)] // One system call that fills a buffer this function owns.
pub fn computer_name() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is writable for its length; the name ends with a nul within it.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } != 0 {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is writable for `len` wide characters.
        if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, buf.as_mut_ptr(), &mut len) } == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
    #[cfg(not(any(unix, windows)))]
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_free_space_of_a_folder_s_disk_is_known() {
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|f| f > 0));
    }

    #[test]
    fn a_folder_s_disk_is_at_least_as_big_as_its_free_space() {
        let dir = std::env::temp_dir();
        assert!(disk_bytes(&dir).unwrap() >= free_bytes(&dir).unwrap());
    }

    #[test]
    fn this_computer_has_a_name() {
        assert!(!computer_name().is_empty());
    }
}
