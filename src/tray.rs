//! Notification-area icon, context menu, and balloon notifications.
//!
//! Balloons come from `Shell_NotifyIcon` rather than the WinRT toast stack:
//! no COM apartment, no AppUserModelID registration, no extra dependency.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_NONE, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, SetForegroundWindow, TrackPopupMenu,
    HICON, HMENU, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

use crate::win::{wide, wide_into};

/// Tray icon callback message.
pub const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

// Menu command identifiers.
pub const CMD_CHASE: u32 = 100;
pub const CMD_WINDOWS: u32 = 101;
pub const CMD_REACT: u32 = 102;
pub const CMD_JUMP: u32 = 113;
pub const CMD_ADD_SPRITE: u32 = 114;
pub const CMD_SLEEP: u32 = 103;
pub const CMD_WAKE: u32 = 104;
pub const CMD_COME: u32 = 105;
pub const CMD_OPEN_DIR: u32 = 106;
pub const CMD_RELOAD: u32 = 107;
pub const CMD_ABOUT: u32 = 108;
pub const CMD_EXIT: u32 = 109;
pub const CMD_CLOSE_MENU: u32 = 110;
pub const CMD_OPEN_SPRITES: u32 = 111;
pub const CMD_EXPORT_SHEET: u32 = 112;
/// Scale commands are `CMD_SCALE_BASE + n` for n in 1..=6.
pub const CMD_SCALE_BASE: u32 = 200;
/// Sprite commands are `CMD_SPRITE_BASE + i`, indexing `MenuState::sprites`.
pub const CMD_SPRITE_BASE: u32 = 300;
/// Roam commands are `CMD_ROAM_BASE + i`, indexing [`ROAM_PRESETS`].
pub const CMD_ROAM_BASE: u32 = 400;
/// Sleep-timer commands are `CMD_SLEEP_BASE + i`, indexing [`SLEEP_PRESETS`].
pub const CMD_SLEEP_BASE: u32 = 500;

/// Idle time before the creature nods off, in seconds. "Never" is the 24 hour
/// clamp ceiling rather than a real infinity — simpler than a special case,
/// and nobody leaves a session running that long.
pub const SLEEP_PRESETS: [(&str, u64); 5] = [
    ("After 1 minute", 60),
    ("After 3 minutes", 180),
    ("After 10 minutes", 600),
    ("After 30 minutes", 1800),
    ("Never", 86_400),
];

fn nearest_sleep(secs: u64) -> usize {
    SLEEP_PRESETS
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, v))| v.abs_diff(secs))
        .map(|(i, _)| i)
        .unwrap_or(1)
}

/// How restless the creature is: label and the `roam` value it sets.
/// Presets rather than a raw number, because "how often does it wander" is a
/// feel, not a figure — the config file still takes any 0-100 value.
pub const ROAM_PRESETS: [(&str, u8); 5] = [
    ("Hardly ever", 5),
    ("Now and then", 25),
    ("Normal", 45),
    ("Often", 70),
    ("Constantly", 95),
];

/// The preset whose value is nearest `roam`, for the checkmark.
fn nearest_roam(roam: u8) -> usize {
    ROAM_PRESETS
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, v))| (*v as i32 - roam as i32).abs())
        .map(|(i, _)| i)
        .unwrap_or(2)
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, icon: HICON) -> Tray {
        let mut t = Tray {
            hwnd,
            icon,
            added: false,
        };
        t.add();
        t
    }

    fn base(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid
    }

    /// (Re)create the icon. Also called when Explorer restarts.
    pub fn add(&mut self) {
        let mut nid = self.base();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.icon;
        wide_into(&mut nid.szTip, "PetPal");
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &nid) != 0 {
                self.added = true;
            }
        }
    }

    pub fn set_icon(&mut self, icon: HICON) {
        self.icon = icon;
        if !self.added {
            return;
        }
        let mut nid = self.base();
        nid.uFlags = NIF_ICON;
        nid.hIcon = icon;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    /// Raise a balloon notification.
    pub fn notify(&self, title: &str, message: &str) {
        if !self.added {
            return;
        }
        let mut nid = self.base();
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_NONE;
        wide_into(&mut nid.szInfoTitle, title);
        wide_into(&mut nid.szInfo, message);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if self.added {
            let nid = self.base();
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
        }
    }
}

pub struct MenuState {
    pub chase: bool,
    pub walk_on_windows: bool,
    pub jump_between_windows: bool,
    pub react: bool,
    pub asleep: bool,
    pub scale: u32,
    /// Current restlessness, 0-100.
    pub roam: u8,
    /// Current idle-before-sleep timer, in seconds.
    pub sleep_after_idle_secs: u64,
    /// Every selectable sprite: the built-in creatures followed by any sheets
    /// installed under `<config>/sprites/`. Index + `CMD_SPRITE_BASE` is the
    /// command id, and `.1` is what gets written to `sprite` in the config.
    pub sprites: Vec<(String, String)>,
    /// The currently active entry in `sprites`.
    pub sprite: String,
}

/// Should the menu reappear after this command, rather than closing?
///
/// Settings stay open so several can be changed in one visit; anything that
/// hands the user off elsewhere (a folder, a dialog) or is plainly terminal
/// closes.
pub fn keeps_menu_open(cmd: u32) -> bool {
    match cmd {
        CMD_CHASE | CMD_WINDOWS | CMD_JUMP | CMD_REACT | CMD_SLEEP | CMD_WAKE | CMD_RELOAD => true,
        // Opening the editor should dismiss the menu; it is a window, not a toggle.
        CMD_ADD_SPRITE => false,
        n if (CMD_SCALE_BASE..CMD_SCALE_BASE + 100).contains(&n) => true,
        n if (CMD_SPRITE_BASE..CMD_ROAM_BASE).contains(&n) => true,
        n if (CMD_ROAM_BASE..CMD_SLEEP_BASE).contains(&n) => true,
        n if n >= CMD_SLEEP_BASE => true,
        _ => false,
    }
}

/// Show the context menu at `anchor` and return the chosen command, or 0 if it
/// was dismissed.
///
/// `TPM_RETURNCMD` hands the id straight back instead of routing a `WM_COMMAND`,
/// which keeps all the menu logic in one place — and lets the caller loop,
/// reopening the menu at the same anchor so it behaves like a small settings
/// panel instead of vanishing on every click.
pub fn show_menu(hwnd: HWND, anchor: POINT, st: &MenuState) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }

        check_item(menu, CMD_CHASE, "Chase the cursor", st.chase);
        check_item(menu, CMD_WINDOWS, "Walk on windows", st.walk_on_windows);
        check_item(menu, CMD_JUMP, "Jump between windows", st.jump_between_windows);
        check_item(menu, CMD_REACT, "React to new apps", st.react);
        sep(menu);

        // Sprite picker.
        let sprite_menu = CreatePopupMenu();
        for (i, (label, id)) in st.sprites.iter().enumerate() {
            check_item(
                sprite_menu,
                CMD_SPRITE_BASE + i as u32,
                label,
                *id == st.sprite,
            );
        }
        sep(sprite_menu);
        item(sprite_menu, CMD_ADD_SPRITE, "Add or edit sprites...");
        item(sprite_menu, CMD_EXPORT_SHEET, "Make a copy to edit...");
        item(sprite_menu, CMD_OPEN_SPRITES, "Open sprites folder...");
        submenu(menu, sprite_menu, "Sprite");

        let size_menu = CreatePopupMenu();
        for n in 1..=6u32 {
            check_item(
                size_menu,
                CMD_SCALE_BASE + n,
                &format!("{n}x"),
                st.scale == n,
            );
        }
        submenu(menu, size_menu, "Size");

        // How often it wanders off on its own.
        let roam_menu = CreatePopupMenu();
        let current = nearest_roam(st.roam);
        for (i, (label, _)) in ROAM_PRESETS.iter().enumerate() {
            check_item(roam_menu, CMD_ROAM_BASE + i as u32, label, i == current);
        }
        submenu(menu, roam_menu, "Roam around");

        // How long it waits before nodding off.
        let sleep_menu = CreatePopupMenu();
        let current_sleep = nearest_sleep(st.sleep_after_idle_secs);
        for (i, (label, _)) in SLEEP_PRESETS.iter().enumerate() {
            check_item(
                sleep_menu,
                CMD_SLEEP_BASE + i as u32,
                label,
                i == current_sleep,
            );
        }
        submenu(menu, sleep_menu, "Sleep when idle");
        sep(menu);

        if st.asleep {
            item(menu, CMD_WAKE, "Wake up");
        } else {
            item(menu, CMD_SLEEP, "Go to sleep");
        }
        item(menu, CMD_COME, "Come here");
        sep(menu);

        item(menu, CMD_OPEN_DIR, "Open config folder...");
        item(menu, CMD_RELOAD, "Reload config && sprites");
        item(menu, CMD_ABOUT, "About PetPal");
        sep(menu);
        item(menu, CMD_CLOSE_MENU, "Close menu");
        sep(menu);
        item(menu, CMD_EXIT, "Exit PetPal");

        // Required so the menu dismisses when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            anchor.x,
            anchor.y,
            0,
            hwnd,
            null_mut(),
        );
        DestroyMenu(menu);
        cmd as u32
    }
}

/// Where the menu should open: the cursor, captured once so that reopening it
/// after each change keeps it anchored instead of drifting with the mouse.
pub fn cursor_anchor() -> POINT {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    pt
}

fn item(menu: HMENU, id: u32, text: &str) {
    let w = wide(text);
    unsafe { AppendMenuW(menu, MF_STRING, id as usize, w.as_ptr()) };
}

fn check_item(menu: HMENU, id: u32, text: &str, checked: bool) {
    let w = wide(text);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe { AppendMenuW(menu, flags, id as usize, w.as_ptr()) };
}

fn sep(menu: HMENU) {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null_mut()) };
}

fn submenu(parent: HMENU, child: HMENU, text: &str) {
    let w = wide(text);
    unsafe { AppendMenuW(parent, MF_POPUP, child as usize, w.as_ptr()) };
}
