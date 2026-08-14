//! Small shared helpers over the Win32 surface: string marshalling, a clock,
//! and a dependency-free PRNG.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

/// NUL-terminated UTF-16 for the `...W` APIs.
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Copy `s` into a fixed-size UTF-16 field, NUL-terminated and truncated to fit.
pub fn wide_into(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in OsStr::new(s).encode_wide() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}

/// Decode a NUL-terminated UTF-16 buffer.
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Say something before dying, instead of just vanishing.
///
/// The crate aborts on panic and is a `windows_subsystem = "windows"` binary, so
/// it has no console: the default panic message goes nowhere at all. What the
/// user sees is the window disappearing, which is indistinguishable from a
/// hang, a crash, and quitting on purpose — and leaves nothing to report.
///
/// The hook still runs ahead of the abort, so one message box and one appended
/// log line turn every future panic into something actionable. Installed first
/// thing in `main`, before any window exists.
pub fn install_panic_reporter(log: std::path::PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let body = format!("PetPal {} hit a bug and has to close.\n\n{msg}\n\nat {at}", env!("CARGO_PKG_VERSION"));

        if let Some(parent) = log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Appending keeps earlier crashes around; a repeat is the useful signal.
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            use std::io::Write;
            let _ = writeln!(f, "[{}ms] {msg} at {at}", now_ms());
        }

        let text = format!("{body}\n\nWritten to:\n{}", log.display());
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(&text).as_ptr(),
                wide("PetPal").as_ptr(),
                MB_ICONERROR | MB_OK,
            );
        }
    }));
}

/// Milliseconds since boot. Monotonic and cheap (no syscall on modern Windows).
#[inline]
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// xorshift64*. We only need jitter for idle behaviour, so a real RNG crate
/// would be dead weight.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        // Seeded off the boot clock; the low bits move fast enough to differ
        // between runs, and a fixed fallback keeps the state non-zero.
        Rng::with_seed(now_ms())
    }

    /// Fixed seed, so behavioural tests are reproducible. A clock-seeded RNG
    /// makes a randomised test flaky, which is worse than having no test.
    pub fn with_seed(seed: u64) -> Self {
        // The low bit keeps the state non-zero, which xorshift requires.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (n > 0).
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next_u64() >> 32) as u32) % n
    }

    /// Uniform in `lo..=hi`.
    #[inline]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    /// True with probability `percent`.
    #[inline]
    pub fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
}
