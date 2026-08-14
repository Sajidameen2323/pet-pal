//! Starting with Windows, via the per-user `Run` key.
//!
//! The `Run` key rather than a scheduled task or a Startup-folder shortcut: it
//! needs no elevation, no COM, no `.lnk` writing, and it is the one place users
//! and every "what starts with my PC?" tool already look — Task Manager's
//! Startup tab reads it, so the toggle here and the switch there control the
//! same thing.
//!
//! **The registry is the setting.** It is deliberately not mirrored into
//! `config.toml`: two copies of one fact drift the moment anything else touches
//! either — Task Manager disabling the entry, a config file copied to another
//! machine, an uninstall — and then the menu shows a checkmark that is a lie.
//! The tick is read back from the key each time the menu opens.

use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ,
};

use crate::win::{from_wide, wide};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Our value under that key. Also what shows in Task Manager's Startup tab.
const VALUE: &str = "PetPal";

/// Full path of the running executable.
pub fn exe_path() -> Option<PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize];
    let n = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    // A truncated path would be a working-looking value that launches nothing.
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    Some(PathBuf::from(from_wide(&buf[..n as usize])))
}

/// The command line to register: the exe path, quoted.
///
/// Quoted because the path routinely contains spaces, and an unquoted `Run`
/// value is split on them — `C:\Program Files\...` would try to launch
/// `C:\Program`.
fn command_for(exe: &PathBuf) -> String {
    format!("\"{}\"", exe.display())
}

/// RAII wrapper so no early return leaks the key.
struct Key(HKEY);

impl Key {
    fn open(access: u32) -> Option<Key> {
        let sub = wide(RUN_KEY);
        let mut h: HKEY = null_mut();
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut h) };
        (rc == ERROR_SUCCESS).then_some(Key(h))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

/// The command currently registered, if any.
fn registered() -> Option<String> {
    let key = Key::open(KEY_READ)?;
    let name = wide(VALUE);
    let mut buf = [0u16; 1024];
    let mut len = std::mem::size_of_val(&buf) as u32;
    let mut kind = 0u32;
    let rc = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            null_mut(),
            &mut kind,
            buf.as_mut_ptr() as *mut u8,
            &mut len,
        )
    };
    if rc != ERROR_SUCCESS || kind != REG_SZ {
        return None;
    }
    // `len` is bytes, and the stored string may or may not include its NUL.
    let chars = (len as usize / 2).min(buf.len());
    Some(from_wide(&buf[..chars]))
}

/// Is PetPal set to start with Windows?
pub fn is_enabled() -> bool {
    registered().is_some()
}

/// Does the registered command point somewhere other than this executable?
///
/// True after the exe is moved or replaced — the entry still exists, so the
/// toggle reads as on, but what it launches is gone. See [`repair`].
fn is_stale() -> bool {
    let (Some(cur), Some(exe)) = (registered(), exe_path()) else {
        return false;
    };
    // Compared case-insensitively and without quotes: Windows paths are
    // case-insensitive, and an entry written by hand may not be quoted.
    let norm = |s: &str| s.trim().trim_matches('"').to_lowercase();
    norm(&cur) != norm(&command_for(&exe))
}

/// Turn starting-with-Windows on or off.
pub fn set(enabled: bool) -> Result<(), String> {
    let key = Key::open(KEY_SET_VALUE)
        .ok_or_else(|| format!("cannot open HKCU\\{RUN_KEY}"))?;
    let name = wide(VALUE);

    if !enabled {
        let rc = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        // Deleting something that was not there is the state we wanted anyway.
        return if rc == ERROR_SUCCESS || registered().is_none() {
            Ok(())
        } else {
            Err(format!("could not remove the startup entry (error {rc})"))
        };
    }

    let exe = exe_path().ok_or("cannot determine where PetPal is running from")?;
    let data = wide(&command_for(&exe));
    let rc = unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr() as *const u8,
            // Bytes, including the terminating NUL that `wide` appends.
            std::mem::size_of_val(&data[..]) as u32,
        )
    };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("could not write the startup entry (error {rc})"))
    }
}

/// Point an existing entry back at this executable if it has drifted.
///
/// Called once at start-up. Moving `petpal.exe` — which is the normal way to
/// install it, since it ships as a bare exe in a repo — otherwise leaves a
/// startup entry aimed at a path that no longer exists: the pet silently stops
/// appearing at login while the menu still says it should. Rewriting it is what
/// the user asked for when they ticked the box, so it is not a surprise.
///
/// Does nothing when the toggle is off, so it can never turn itself on.
pub fn repair() {
    if is_enabled() && is_stale() {
        let _ = set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path with spaces is the normal case on Windows, and an unquoted `Run`
    /// value is split on them — `"C:\Program Files\PetPal\petpal.exe"` without
    /// the quotes launches `C:\Program`.
    #[test]
    fn the_registered_command_is_quoted() {
        let p = PathBuf::from(r"C:\Program Files\PetPal\petpal.exe");
        let cmd = command_for(&p);
        assert!(cmd.starts_with('"') && cmd.ends_with('"'), "not quoted: {cmd}");
        assert!(cmd.contains(r"Program Files\PetPal"));
        // Exactly the two we added, so nothing is doubly quoted.
        assert_eq!(cmd.matches('"').count(), 2);
    }

    /// Whatever this process is, asking where it runs from has to work — it is
    /// the value the toggle writes.
    #[test]
    fn the_executable_path_is_discoverable() {
        let exe = exe_path().expect("GetModuleFileNameW should give a path");
        assert!(exe.is_absolute(), "{exe:?} is not absolute");
        assert!(exe.exists(), "{exe:?} does not exist");
    }

    /// Reading the toggle must be safe and quiet whatever the key contains,
    /// including when our value is absent — which is the state on a machine
    /// that has never enabled it.
    #[test]
    fn reading_the_toggle_never_panics() {
        let _ = is_enabled();
        let _ = is_stale();
        // And it agrees with itself.
        assert_eq!(is_enabled(), registered().is_some());
    }

    /// Puts back whatever was under the `Run` key, however the test ends.
    struct Restore(Option<String>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let Some(key) = Key::open(KEY_SET_VALUE) else { return };
            let name = wide(VALUE);
            match &self.0 {
                Some(prev) => {
                    let data = wide(prev);
                    unsafe {
                        RegSetValueExW(
                            key.0,
                            name.as_ptr(),
                            0,
                            REG_SZ,
                            data.as_ptr() as *const u8,
                            std::mem::size_of_val(&data[..]) as u32,
                        )
                    };
                }
                None => {
                    unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
                }
            }
        }
    }

    /// The round trip against the real `Run` key, which is the only thing that
    /// proves the toggle does what the menu claims. Writes to `HKCU\...\Run`
    /// and puts the previous state back, so it is developer tooling rather
    /// than part of the suite:
    ///
    /// ```text
    /// cargo test -- --ignored --test-threads=1 the_toggle_round_trips
    /// ```
    ///
    /// `--test-threads=1` is not optional: this and
    /// [`a_moved_executable_is_repaired`] both drive the same one registry
    /// value, and run in parallel they clobber each other's setup.
    #[test]
    #[ignore]
    fn the_toggle_round_trips() {
        let _restore = Restore(registered());

        set(false).expect("clearing should succeed");
        assert!(!is_enabled(), "should read as off after clearing");
        // Off twice is not an error; the state asked for is the state reached.
        set(false).expect("clearing an already-clear entry is not a failure");

        set(true).expect("enabling should succeed");
        assert!(is_enabled(), "should read as on after enabling");
        let cmd = registered().expect("a command should be registered");
        let exe = exe_path().unwrap();
        assert_eq!(cmd, command_for(&exe), "should register this exe, quoted");
        assert!(!is_stale(), "a freshly written entry is not stale");

        set(false).expect("disabling should succeed");
        assert!(!is_enabled(), "should read as off again");
        println!("round trip ok; registered command was {cmd}");
    }

    /// An entry left pointing at an old location should be recognised as stale
    /// and repaired, rather than sitting there launching nothing. Shares the
    /// registry value with [`the_toggle_round_trips`], so run both under
    /// `--test-threads=1`.
    #[test]
    #[ignore]
    fn a_moved_executable_is_repaired() {
        let _restore = Restore(registered());

        // Stand in for "the exe used to live somewhere else".
        let key = Key::open(KEY_SET_VALUE).expect("Run key");
        let name = wide(VALUE);
        let data = wide("\"C:\\nowhere\\petpal.exe\"");
        unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                std::mem::size_of_val(&data[..]) as u32,
            )
        };
        drop(key);

        assert!(is_enabled(), "the entry exists, so the toggle reads as on");
        assert!(is_stale(), "and it points somewhere else");

        repair();
        assert!(!is_stale(), "repair should have pointed it back here");
        assert_eq!(registered().unwrap(), command_for(&exe_path().unwrap()));

        // With the toggle off, repair must never resurrect it.
        set(false).unwrap();
        repair();
        assert!(!is_enabled(), "repair must not turn the toggle on");
    }
}
