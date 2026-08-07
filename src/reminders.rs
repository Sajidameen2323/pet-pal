//! Wall-clock reminders. The creature delivers them by perking up and raising
//! a tray balloon.

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::config::Config;

/// How often to consult the clock. Reminders have minute resolution, so this
/// is plenty and keeps the idle path free of work.
const CHECK_INTERVAL_MS: u64 = 5_000;

/// If we were away longer than this (machine sleep, hibernate), skip the
/// missed window rather than dumping a day of reminders at once.
const MAX_CATCHUP_MIN: i32 = 5;

pub struct Scheduler {
    next_check: u64,
    /// Minute-of-day we last evaluated, or -1 before the first check.
    last_minute: i32,
    /// Date stamp of `last_minute`, so the day rollover is unambiguous.
    last_date: u32,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        Scheduler {
            next_check: 0,
            last_minute: -1,
            last_date: 0,
        }
    }

    /// Returns the text of a reminder that has just come due, if any.
    /// Call repeatedly; each due reminder is returned exactly once.
    pub fn poll(&mut self, now: u64, cfg: &Config) -> Option<String> {
        if cfg.reminders.is_empty() {
            return None;
        }
        if now < self.next_check {
            return None;
        }
        self.next_check = now + CHECK_INTERVAL_MS;

        let st = local_time();
        let minute = st.wHour as i32 * 60 + st.wMinute as i32;
        let date = (st.wYear as u32) << 9 | (st.wMonth as u32) << 5 | st.wDay as u32;

        let prev = if self.last_date == date {
            self.last_minute
        } else {
            // First check of the day (or the first check ever): only consider
            // the current minute, never the whole day behind us.
            minute - 1
        };
        self.last_date = date;
        self.last_minute = minute;

        if minute <= prev {
            return None;
        }
        let from = if minute - prev > MAX_CATCHUP_MIN {
            minute - 1
        } else {
            prev
        };

        for r in &cfg.reminders {
            let Some(m) = parse_hhmm(&r.at) else { continue };
            if m <= from || m > minute {
                continue;
            }
            if !day_matches(&r.days, st.wDayOfWeek) {
                continue;
            }
            return Some(r.text.clone());
        }
        None
    }
}

fn local_time() -> SYSTEMTIME {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    }
}

/// Parse `"HH:MM"` into a minute-of-day.
fn parse_hhmm(s: &str) -> Option<i32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: i32 = h.trim().parse().ok()?;
    let m: i32 = m.trim().parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(h * 60 + m)
}

/// `wday` is 0 = Sunday, matching `SYSTEMTIME`. An empty list means every day.
fn day_matches(days: &[String], wday: u16) -> bool {
    if days.is_empty() {
        return true;
    }
    const NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let today = NAMES[(wday as usize).min(6)];
    days.iter().any(|d| {
        let d = d.trim().to_ascii_lowercase();
        d.starts_with(today) || (d == "weekday" && (1..=5).contains(&wday)) || (d == "weekend" && (wday == 0 || wday == 6))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("14:30"), Some(870));
        assert_eq!(parse_hhmm(" 9:05 "), Some(545));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("noon"), None);
    }

    #[test]
    fn day_filters() {
        let none: Vec<String> = vec![];
        assert!(day_matches(&none, 3));
        assert!(day_matches(&["wed".into()], 3));
        assert!(day_matches(&["wednesday".into()], 3));
        assert!(!day_matches(&["wed".into()], 4));
        assert!(day_matches(&["weekday".into()], 1));
        assert!(!day_matches(&["weekday".into()], 0));
        assert!(day_matches(&["weekend".into()], 6));
    }
}
