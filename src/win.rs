//! Small shared helpers over the Win32 surface: string marshalling, a clock,
//! and a dependency-free PRNG.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

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
        Rng(now_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
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
