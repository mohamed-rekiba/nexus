//! The local time for the top bar's clock, as the old launcher showed it: "9:38 PM".
//!
//! The standard library has no time zones, so this asks the system: `localtime_r` on Unix,
//! `GetLocalTime` on Windows.

/// The local hour (0 to 23) and minute.
#[allow(unsafe_code)] // Two system calls that fill a struct this function owns; nothing else.
fn local_hm() -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        let now = libc::time_t::try_from(now).ok()?;
        // SAFETY: `tm` is a plain struct, zeroed, and `localtime_r` writes into it only.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers are valid for the call; the reentrant form keeps no state.
        let filled = unsafe { libc::localtime_r(&now, &mut tm) };
        if filled.is_null() {
            return None;
        }
        Some((u32::try_from(tm.tm_hour).ok()?, u32::try_from(tm.tm_min).ok()?))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: a plain struct, zeroed; `GetLocalTime` fills it and can't fail.
        let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut now) };
        Some((u32::from(now.wHour), u32::from(now.wMinute)))
    }
}

/// "9:38 PM", or empty when the system doesn't say.
pub fn now() -> String {
    local_hm().map(|(h, m)| text(h, m)).unwrap_or_default()
}

fn text(hour: u32, minute: u32) -> String {
    let twelve = if hour.is_multiple_of(12) { 12 } else { hour % 12 };
    format!("{twelve}:{minute:02} {}", if hour < 12 { "AM" } else { "PM" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_as_twelve_hours() {
        assert_eq!(text(0, 5), "12:05 AM");
        assert_eq!(text(9, 38), "9:38 AM");
        assert_eq!(text(12, 0), "12:00 PM");
        assert_eq!(text(21, 38), "9:38 PM");
    }

    #[test]
    fn the_system_gives_a_time() {
        assert!(!now().is_empty());
    }
}
