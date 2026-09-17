pub mod api;
#[cfg(windows)]
pub mod windows;
pub fn timestamp_string(t: std::time::SystemTime) -> String {
    let (y, mo, d, h, mi) = local_datetime(t);
    format!("{y:04}/{mo:02}/{d:02} {h:02}:{mi:02}")
}
#[cfg(windows)]
fn local_datetime(t: std::time::SystemTime) -> (u32, u32, u32, u32, u32) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime(u32, u32);
    #[repr(C)]
    struct SystemTime(u16, u16, u16, u16, u16, u16, u16, u16);
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToLocalFileTime(
            lpFileTime: *const FileTime,
            lpLocalFileTime: *mut FileTime,
        ) -> i32;
        fn FileTimeToSystemTime(
            lpFileTime: *const FileTime,
            lpSystemTime: *mut SystemTime,
        ) -> i32;
    }
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let ticks = dur.as_nanos() / 100 + 116_444_736_000_000_000u128;
    let ft = FileTime(ticks as u32, (ticks >> 32) as u32);
    let mut local = ft;
    let mut st = SystemTime(0, 0, 0, 0, 0, 0, 0, 0);
    unsafe {
        if FileTimeToLocalFileTime(&ft, &mut local) == 0
            || FileTimeToSystemTime(&local, &mut st) == 0
        {
            return (1970, 1, 1, 0, 0);
        }
    }
    (st.0 as u32, st.1 as u32, st.3 as u32, st.4 as u32, st.5 as u32)
}
#[cfg(not(windows))]
fn local_datetime(t: std::time::SystemTime) -> (u32, u32, u32, u32, u32) {
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let (y, mo, d) = civil_from_days(days as i64);
    (y as u32, mo, d, ((secs % 86400) / 3600) as u32, ((secs % 3600) / 60) as u32)
}
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
