//! Discovering what the creature can stand on: the top edges of visible
//! windows, the taskbar, and each monitor's work area floor.
//!
//! Window edges are clipped against everything stacked above them, so the pet
//! never stands on a title bar that is actually hidden behind another window.

use std::ptr::null_mut;

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetClassNameW, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use crate::win::{from_wide, wide};

/// A horizontal surface the creature can stand on.
#[derive(Clone, Copy, Debug)]
pub struct Ledge {
    pub x0: i32,
    pub x1: i32,
    pub y: i32,
}

impl Ledge {
    #[inline]
    pub fn holds(&self, x: i32) -> bool {
        x >= self.x0 && x <= self.x1
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }
}

#[derive(Clone, Copy)]
pub struct Monitor {
    pub rect: RECT,
    pub work: RECT,
}

/// Ledges narrower than this are not worth standing on and just make the pet
/// jitter between surfaces.
const MIN_LEDGE_WIDTH: i32 = 48;
/// Bounds on work per scan. Desktops with hundreds of windows stay cheap.
const MAX_WINDOWS: usize = 40;
const MAX_LEDGES: usize = 96;
/// Windows smaller than this are dialogs and popups; skip them as scenery.
const MIN_WINDOW_W: i32 = 140;
const MIN_WINDOW_H: i32 = 80;

pub struct World {
    pub monitors: Vec<Monitor>,
    pub ledges: Vec<Ledge>,
    next_scan_ms: u64,
    next_monitor_scan_ms: u64,
    self_hwnd: HWND,
}

/// Scratch state threaded through the `EnumWindows` callback.
struct ScanCtx {
    self_hwnd: HWND,
    /// Window rects already seen, i.e. those stacked *above* the current one.
    occluders: Vec<RECT>,
    ledges: Vec<Ledge>,
    /// Reused interval buffers for the occlusion subtraction.
    segs: Vec<(i32, i32)>,
    next: Vec<(i32, i32)>,
}

impl World {
    pub fn new(self_hwnd: HWND) -> World {
        let mut w = World {
            monitors: Vec::new(),
            ledges: Vec::new(),
            next_scan_ms: 0,
            next_monitor_scan_ms: 0,
            self_hwnd,
        };
        w.scan_monitors();
        w
    }

    /// Rescan window ledges, rate-limited to `interval_ms` unless forced.
    pub fn refresh(&mut self, now: u64, interval_ms: u64, force: bool) {
        if now >= self.next_monitor_scan_ms || force {
            self.next_monitor_scan_ms = now + 5_000;
            self.scan_monitors();
        }
        if now < self.next_scan_ms && !force {
            return;
        }
        self.next_scan_ms = now + interval_ms;
        self.scan_ledges();
    }

    /// Build a world with fixed geometry, so behaviour can be tested without a
    /// desktop.
    #[cfg(test)]
    pub fn for_test(monitors: Vec<Monitor>, ledges: Vec<Ledge>) -> World {
        World {
            monitors,
            ledges,
            next_scan_ms: u64::MAX,
            next_monitor_scan_ms: u64::MAX,
            self_hwnd: null_mut(),
        }
    }

    /// Drop all window ledges, leaving only monitor floors. Used when the user
    /// turns window-walking off so the pet falls back to the desktop.
    pub fn floors_only(&mut self) {
        self.ledges.clear();
        self.push_floors();
    }

    fn scan_monitors(&mut self) {
        self.monitors.clear();
        unsafe {
            EnumDisplayMonitors(
                null_mut(),
                null_mut(),
                Some(monitor_proc),
                &mut self.monitors as *mut Vec<Monitor> as LPARAM,
            );
        }
        if self.monitors.is_empty() {
            // Extremely defensive: without a monitor there is nowhere to stand.
            self.monitors.push(Monitor {
                rect: RECT {
                    left: 0,
                    top: 0,
                    right: 1024,
                    bottom: 768,
                },
                work: RECT {
                    left: 0,
                    top: 0,
                    right: 1024,
                    bottom: 728,
                },
            });
        }
    }

    fn push_floors(&mut self) {
        // The work area already excludes the taskbar, so its bottom edge is
        // exactly "standing on the taskbar" when one is present.
        for m in &self.monitors {
            self.ledges.push(Ledge {
                x0: m.work.left,
                x1: m.work.right,
                y: m.work.bottom,
            });
        }
    }

    fn scan_ledges(&mut self) {
        let mut ctx = ScanCtx {
            self_hwnd: self.self_hwnd,
            occluders: Vec::with_capacity(MAX_WINDOWS),
            ledges: Vec::with_capacity(MAX_LEDGES),
            segs: Vec::with_capacity(8),
            next: Vec::with_capacity(8),
        };

        // The taskbar sits above ordinary windows, so seed it as the first
        // occluder; its own top edge is covered by the work-area floor.
        for tb in taskbars() {
            ctx.occluders.push(tb);
        }

        unsafe {
            EnumWindows(Some(enum_proc), &mut ctx as *mut ScanCtx as LPARAM);
        }

        self.ledges.clear();
        self.ledges.append(&mut ctx.ledges);
        self.push_floors();
    }

    pub fn monitor_at(&self, x: i32, y: i32) -> Monitor {
        for m in &self.monitors {
            if x >= m.rect.left && x < m.rect.right && y >= m.rect.top && y < m.rect.bottom {
                return *m;
            }
        }
        // Off-screen (e.g. after a monitor unplug): snap to the nearest.
        unsafe {
            let h = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            if let Some(m) = monitor_info(h) {
                return m;
            }
        }
        self.monitors[0]
    }

    /// The highest surface at or below `feet_y` under `x`, i.e. what the pet
    /// would land on if it fell from here.
    pub fn ground_below(&self, x: i32, feet_y: i32) -> Option<Ledge> {
        let mut best: Option<Ledge> = None;
        for l in &self.ledges {
            if !l.holds(x) || l.y < feet_y {
                continue;
            }
            if best.is_none_or(|b| l.y < b.y) {
                best = Some(*l);
            }
        }
        best
    }

    /// The ledge currently supporting feet at `(x, y)`, within `tol` pixels.
    pub fn support_at(&self, x: i32, y: i32, tol: i32) -> Option<Ledge> {
        let mut best: Option<Ledge> = None;
        for l in &self.ledges {
            if !l.holds(x) || (l.y - y).abs() > tol {
                continue;
            }
            if best.is_none_or(|b| (l.y - y).abs() < (b.y - y).abs()) {
                best = Some(*l);
            }
        }
        best
    }

    /// Is there anything to land on below `(x, y)`? Guards deliberate hops off
    /// an edge, so the pet never steps into a void and has to be teleported back.
    pub fn has_ledge_below(&self, x: i32, y: i32, max_dx: i32) -> bool {
        self.ledges
            .iter()
            .any(|l| l.y > y + 24 && (x.clamp(l.x0, l.x1) - x).abs() <= max_dx)
    }

    /// A ledge below `(x, y)` that the creature could deliberately hop down to.
    ///
    /// The mirror of [`Self::ledge_above`]. Without it the pet can only ever
    /// climb, and ends up stranded on whatever the topmost window happens to be.
    pub fn ledge_below(
        &self,
        x: i32,
        y: i32,
        max_drop: i32,
        max_dx: i32,
        min_width: i32,
        pick: u32,
    ) -> Option<Ledge> {
        // A drop is only worth taking if it lands somewhere clearly lower; a
        // few pixels down is a stumble, not a hop.
        let reachable = |l: &Ledge| {
            let drop = l.y - y;
            drop >= 24
                && drop <= max_drop
                && l.width() >= min_width
                && (x.clamp(l.x0, l.x1) - x).abs() <= max_dx
        };
        let n = self.ledges.iter().filter(|l| reachable(l)).count();
        if n == 0 {
            return None;
        }
        // Prefer the nearest surface below: dropping past two windows onto the
        // taskbar when there was a window right underneath looks like a miss.
        let mut candidates: Vec<&Ledge> = self.ledges.iter().filter(|l| reachable(l)).collect();
        candidates.sort_by_key(|l| l.y);
        let nearest_y = candidates[0].y;
        let close: Vec<&Ledge> = candidates
            .into_iter()
            .filter(|l| l.y - nearest_y <= 24)
            .collect();
        close.get(pick as usize % close.len()).map(|l| **l)
    }

    /// A ledge above `(x, y)` that the creature could jump up to.
    ///
    /// `pick` selects among the candidates so the pet does not always hop to
    /// the same window; counting first avoids allocating a candidate list.
    pub fn ledge_above(
        &self,
        x: i32,
        y: i32,
        max_rise: i32,
        max_dx: i32,
        min_width: i32,
        pick: u32,
    ) -> Option<Ledge> {
        let reachable = |l: &Ledge| {
            let rise = y - l.y;
            rise >= 24
                && rise <= max_rise
                && l.width() >= min_width
                && (x.clamp(l.x0, l.x1) - x).abs() <= max_dx
        };
        let n = self.ledges.iter().filter(|l| reachable(l)).count();
        if n == 0 {
            return None;
        }
        self.ledges
            .iter()
            .filter(|l| reachable(l))
            .nth(pick as usize % n)
            .copied()
    }

    /// Nearest ledge to a point, preferring wide ones — used to pick a bed.
    pub fn nearest_ledge(&self, x: i32, y: i32) -> Option<Ledge> {
        let mut best: Option<(i64, Ledge)> = None;
        for l in &self.ledges {
            if l.width() < MIN_LEDGE_WIDTH {
                continue;
            }
            let cx = x.clamp(l.x0, l.x1);
            let dx = (cx - x) as i64;
            let dy = (l.y - y) as i64;
            let d = dx * dx + dy * dy;
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, *l));
            }
        }
        best.map(|(_, l)| l)
    }
}

// ---------------------------------------------------------------------------
// Enumeration callbacks
// ---------------------------------------------------------------------------

unsafe extern "system" fn monitor_proc(
    h: HMONITOR,
    _dc: windows_sys::Win32::Graphics::Gdi::HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = unsafe { &mut *(lparam as *mut Vec<Monitor>) };
    if let Some(m) = monitor_info(h) {
        out.push(m);
    }
    TRUE
}

fn monitor_info(h: HMONITOR) -> Option<Monitor> {
    if h.is_null() {
        return None;
    }
    unsafe {
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(h, &mut mi) == 0 {
            return None;
        }
        Some(Monitor {
            rect: mi.rcMonitor,
            work: mi.rcWork,
        })
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam as *mut ScanCtx) };
    if ctx.occluders.len() >= MAX_WINDOWS || ctx.ledges.len() >= MAX_LEDGES {
        return 0; // stop enumerating
    }
    if std::ptr::eq(hwnd, ctx.self_hwnd) {
        return TRUE;
    }
    let Some(rc) = standable_rect(hwnd) else {
        return TRUE;
    };

    // Subtract every higher window from this one's top edge.
    ctx.segs.clear();
    ctx.segs.push((rc.left, rc.right));
    let y = rc.top;
    for occ in &ctx.occluders {
        if ctx.segs.is_empty() {
            break;
        }
        // Only windows that actually cover this scanline can hide the edge.
        if y < occ.top || y > occ.bottom {
            continue;
        }
        subtract_span(&mut ctx.segs, &mut ctx.next, occ.left, occ.right);
    }

    for &(a, b) in &ctx.segs {
        if b - a >= MIN_LEDGE_WIDTH && ctx.ledges.len() < MAX_LEDGES {
            ctx.ledges.push(Ledge { x0: a, x1: b, y });
        }
    }

    ctx.occluders.push(rc);
    TRUE
}

/// Remove `[lo, hi)` from a set of disjoint, ascending spans.
///
/// This is what stops the creature standing on a title bar that is hidden
/// behind another window: a window's top edge is clipped by every window above
/// it, and one occluder can split a span into two.
///
/// `scratch` is a caller-owned buffer, reused across calls so a scan does not
/// allocate.
fn subtract_span(segs: &mut Vec<(i32, i32)>, scratch: &mut Vec<(i32, i32)>, lo: i32, hi: i32) {
    scratch.clear();
    for &(a, b) in segs.iter() {
        if hi <= a || lo >= b {
            scratch.push((a, b)); // no overlap
            continue;
        }
        if lo > a {
            scratch.push((a, lo));
        }
        if hi < b {
            scratch.push((hi, b));
        }
    }
    std::mem::swap(segs, scratch);
}

// ---------------------------------------------------------------------------
// Window classification
// ---------------------------------------------------------------------------

/// The visible bounds of a window the creature may stand on, or `None` if the
/// window is not real scenery (hidden, minimised, a tool window, the desktop...).
pub fn standable_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return None;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return None; // untitled helper windows
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            return None;
        }

        // UWP apps keep hidden "cloaked" windows around that still report a
        // valid rect; DWM is the only reliable way to spot them.
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            4,
        ) == 0
            && cloaked != 0
        {
            return None;
        }

        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if n > 0 {
            let name = from_wide(&class[..n as usize]);
            if matches!(
                name.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow"
            ) {
                return None;
            }
        }

        // GetWindowRect includes the invisible resize border, which would leave
        // the pet floating a few pixels off the visual edge.
        let mut rc: RECT = std::mem::zeroed();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rc as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        ) != 0
            && GetWindowRect(hwnd, &mut rc) == 0
        {
            return None;
        }

        if rc.right - rc.left < MIN_WINDOW_W || rc.bottom - rc.top < MIN_WINDOW_H {
            return None;
        }
        Some(rc)
    }
}

/// Title of a window, for "app opened" reactions.
pub fn window_title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 128];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return String::new();
        }
        from_wide(&buf[..n as usize])
    }
}

/// Rects of the primary and any secondary taskbars.
fn taskbars() -> Vec<RECT> {
    let mut out = Vec::with_capacity(2);
    let mut push = |hwnd: HWND| {
        if hwnd.is_null() {
            return;
        }
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rc) != 0 {
                out.push(rc);
            }
        }
    };

    let primary = wide("Shell_TrayWnd");
    let secondary = wide("Shell_SecondaryTrayWnd");
    unsafe {
        push(FindWindowExW(
            null_mut(),
            null_mut(),
            primary.as_ptr(),
            null_mut(),
        ));
        let mut prev: HWND = null_mut();
        loop {
            let h = FindWindowExW(null_mut(), prev, secondary.as_ptr(), null_mut());
            if h.is_null() {
                break;
            }
            push(h);
            prev = h;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(spans: &[(i32, i32)], lo: i32, hi: i32) -> Vec<(i32, i32)> {
        let mut segs = spans.to_vec();
        let mut scratch = Vec::new();
        subtract_span(&mut segs, &mut scratch, lo, hi);
        segs
    }

    #[test]
    fn occluder_in_the_middle_splits_the_ledge() {
        assert_eq!(sub(&[(0, 100)], 40, 60), vec![(0, 40), (60, 100)]);
    }

    #[test]
    fn occluder_at_an_end_trims_it() {
        assert_eq!(sub(&[(0, 100)], -20, 30), vec![(30, 100)]);
        assert_eq!(sub(&[(0, 100)], 70, 200), vec![(0, 70)]);
    }

    #[test]
    fn full_cover_removes_the_ledge() {
        assert!(sub(&[(0, 100)], -5, 105).is_empty());
        assert!(sub(&[(0, 100)], 0, 100).is_empty());
    }

    #[test]
    fn non_overlapping_occluder_is_ignored() {
        assert_eq!(sub(&[(0, 100)], 100, 200), vec![(0, 100)]);
        assert_eq!(sub(&[(0, 100)], -50, 0), vec![(0, 100)]);
    }

    #[test]
    fn applies_across_several_spans() {
        // A second occluder cutting an already-split ledge.
        let once = sub(&[(0, 300)], 100, 150);
        assert_eq!(once, vec![(0, 100), (150, 300)]);
        let twice = sub(&once, 200, 250);
        assert_eq!(twice, vec![(0, 100), (150, 200), (250, 300)]);
    }

    #[test]
    fn ledge_queries_respect_span_and_direction() {
        let world = World {
            monitors: vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1000, bottom: 1000 },
                work: RECT { left: 0, top: 0, right: 1000, bottom: 960 },
            }],
            ledges: vec![
                Ledge { x0: 0, x1: 1000, y: 960 },   // floor
                Ledge { x0: 100, x1: 400, y: 700 },  // a window near us
                Ledge { x0: 800, x1: 900, y: 300 },  // far away and high
            ],
            next_scan_ms: 0,
            next_monitor_scan_ms: 0,
            self_hwnd: std::ptr::null_mut(),
        };

        // Falling from above a window lands on the window, not the floor.
        assert_eq!(world.ground_below(200, 100).map(|l| l.y), Some(700));
        // Outside its span, the floor catches us instead.
        assert_eq!(world.ground_below(600, 100).map(|l| l.y), Some(960));
        // Nothing below the floor.
        assert!(world.ground_below(200, 961).is_none());

        // Standing on the floor directly under the window, it is reachable.
        assert_eq!(world.ledge_above(200, 960, 400, 400, 50, 0).map(|l| l.y), Some(700));
        // From further along the floor it is 400px away horizontally: in reach
        // with a generous limit, out of reach with a tight one.
        assert_eq!(world.ledge_above(800, 960, 400, 400, 50, 0).map(|l| l.y), Some(700));
        assert!(world.ledge_above(800, 960, 400, 50, 50, 0).is_none());
        // Too high to jump to.
        assert!(world.ledge_above(200, 960, 100, 400, 50, 0).is_none());
        // Narrower than we insist on.
        assert!(world.ledge_above(200, 960, 400, 400, 500, 0).is_none());
        // Nothing above the highest ledge.
        assert!(world.ledge_above(850, 300, 400, 400, 50, 0).is_none());

        // From the window there is somewhere to drop to; from the floor there isn't.
        assert!(world.has_ledge_below(200, 700, 400));
        assert!(!world.has_ledge_below(200, 960, 400));
    }
}
