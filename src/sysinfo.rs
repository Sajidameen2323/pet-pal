//! Cheap system observation: CPU load, user idle time, and our own footprint.
//!
//! Everything here is a plain syscall on a slow timer — no background threads,
//! no WMI, no performance counters.

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetSystemTimes};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

use crate::win::now_ms;

#[inline]
fn ft(f: &FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Whole-machine CPU load, sampled from kernel tick counters.
pub struct CpuMonitor {
    prev_idle: u64,
    prev_total: u64,
    next_sample: u64,
    /// Smoothed load, 0.0..=1.0.
    pub load: f32,
    period_ms: u64,
}

impl CpuMonitor {
    pub fn new(period_ms: u64) -> CpuMonitor {
        let (idle, total) = read_times();
        CpuMonitor {
            prev_idle: idle,
            prev_total: total,
            next_sample: now_ms() + period_ms,
            load: 0.0,
            period_ms: period_ms.max(250),
        }
    }

    /// Re-sample if the period has elapsed. Returns true when `load` changed.
    pub fn poll(&mut self, now: u64) -> bool {
        if now < self.next_sample {
            return false;
        }
        self.next_sample = now + self.period_ms;

        let (idle, total) = read_times();
        let d_idle = idle.saturating_sub(self.prev_idle);
        let d_total = total.saturating_sub(self.prev_total);
        self.prev_idle = idle;
        self.prev_total = total;
        if d_total == 0 {
            return false;
        }

        let instant = 1.0 - (d_idle as f32 / d_total as f32);
        // Light exponential smoothing so a single busy tick does not set the
        // creature off, and a brief dip does not immediately calm it down.
        self.load += (instant.clamp(0.0, 1.0) - self.load) * 0.5;
        true
    }
}

fn read_times() -> (u64, u64) {
    unsafe {
        let mut idle: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return (0, 0);
        }
        // Kernel time already includes idle time.
        (ft(&idle), ft(&kernel) + ft(&user))
    }
}

/// Milliseconds since the last keyboard or mouse input, machine-wide.
pub fn idle_ms() -> u64 {
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut lii) == 0 {
            return 0;
        }
        // dwTime is a 32-bit tick count; compare in the same width to stay
        // correct across the ~49 day wrap.
        (now_ms() as u32).wrapping_sub(lii.dwTime) as u64
    }
}

/// Our own working set, for the About box.
pub fn working_set_bytes() -> u64 {
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) == 0 {
            return 0;
        }
        pmc.WorkingSetSize as u64
    }
}
