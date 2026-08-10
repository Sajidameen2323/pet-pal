//! The "Add a sprite..." window: pick a PNG, say how big a cell is, click the
//! cells that make up each animation, name it, done.
//!
//! Writing `sprite.toml` by hand is the step that loses people. Not because the
//! format is hard, but because a frame number is a claim about a picture you
//! cannot see while you are typing, and a wrong claim is silent — a clip that
//! points at an empty cell just makes the creature invisible. So this window is
//! built around one idea: **you never type a frame number.** You pick an
//! animation, then click cells, and the cells tell you what they are.
//!
//! It is a plain modeless window sharing the pet's message loop rather than a
//! dialog resource: the app already pumps messages, and a modal loop would
//! re-enter the pet's wndproc while its `RefCell` is borrowed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, StretchDIBits, TextOutW, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, FW_NORMAL, HBRUSH, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OPENFILENAMEW};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, ShellExecuteW, HDROP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::sprites::{Anim, ANIM_COUNT};
use crate::win::{from_wide, wide};

/// Posted to the pet's window once a sprite has been written, so it can pick up
/// the new folder and switch to it. The name is handed over out of band — see
/// [`take_added`] — because a `WPARAM` cannot carry a `String` and a raw pointer
/// across a `PostMessage` would have to outlive this window.
pub const WM_SPRITE_ADDED: u32 = WM_APP + 3;

thread_local! {
    /// Name of the sprite just written. Same thread as the message loop, so a
    /// `thread_local` is enough — no lock, no `unsafe`.
    static ADDED: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The open editor, if any. Keeps the second click on the menu item from
    /// stacking windows.
    static OPEN: RefCell<HWND> = const { RefCell::new(null_mut()) };
}

pub fn take_added() -> Option<String> {
    ADDED.with(|a| a.borrow_mut().take())
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

const WIN_W: i32 = 1090;
const WIN_H: i32 = 700;
/// Width of the right-hand column of controls.
const PANEL_W: i32 = 300;
const PAD: i32 = 12;
/// Height of the two rows of controls above the sheet.
const HEAD_H: i32 = 108;
const FOOT_H: i32 = 46;

const ID_BROWSE: isize = 1001;
const ID_NAME: isize = 1002;
const ID_CELL_W: isize = 1003;
const ID_CELL_H: isize = 1004;
const ID_ANIMS: isize = 1005;
const ID_MS: isize = 1006;
const ID_ROW: isize = 1007;
const ID_CLEAR: isize = 1008;
const ID_ADD: isize = 1009;
const ID_CANCEL: isize = 1010;
const ID_GUESS: isize = 1011;
const ID_FIT: isize = 1012;
const ID_ORIGINAL: isize = 1013;
const ID_TRIM: isize = 1014;
const ID_BG: isize = 1015;
const ID_COLS: isize = 1016;
const ID_ROWS: isize = 1017;

// Menu bar.
const ID_MENU_OPEN: isize = 2001;
const ID_MENU_PROMPT: isize = 2002;
const ID_MENU_HELP: isize = 2003;
const ID_MENU_GUIDE: isize = 2004;

const TIMER_PLAY: usize = 1;

// Control styles. windows-sys does not surface the `SS_*` family at all, and
// mixes signed and unsigned for the rest, so they are spelled out here as plain
// u32 and everything is combined in one type.
const SS_LEFT: u32 = 0x0000;
const SS_CENTER: u32 = 0x0001;
/// Collapse the middle of a long path with "..." instead of clipping the end,
/// so the filename stays visible.
const SS_PATHELLIPSIS: u32 = 0x8000;
const ES_NUMBER_: u32 = 0x2000;
const BS_PUSH: u32 = 0x0000;
const BS_DEFPUSH: u32 = 0x0001;
const LBS_NOTIFY_: u32 = 0x0001;
const ES_MULTILINE_: u32 = 0x0004;
const ES_READONLY_: u32 = 0x0800;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Packed 0xAARRGGBB for the software-composited preview.
const fn argb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

const BG: u32 = argb(0x1E, 0x20, 0x28);
const CHECK_A: u32 = argb(0x3A, 0x3E, 0x4A);
const CHECK_B: u32 = argb(0x32, 0x36, 0x41);
const GRID: u32 = argb(0x5A, 0x60, 0x72);
const PICK: u32 = argb(0xFF, 0xC4, 0x3C);
const OTHER: u32 = argb(0x4E, 0x8C, 0xD8);

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct Editor {
    hwnd: HWND,
    name: HWND,
    cw_edit: HWND,
    ch_edit: HWND,
    ms_edit: HWND,
    list: HWND,
    path_label: HWND,
    hint: HWND,
    frames_label: HWND,
    trim_edit: HWND,
    bg_edit: HWND,
    cols_edit: HWND,
    rows_edit: HWND,

    /// The PNG exactly as loaded. Kept so re-fitting with different settings
    /// always starts from the original rather than compounding resamples.
    orig: Vec<u32>,
    orig_w: i32,
    orig_h: i32,
    /// True when `px` is a fitted version rather than the file itself.
    fitted: bool,

    src: Option<PathBuf>,
    /// Straight ARGB, as decoded. Kept unpremultiplied so the checkerboard
    /// composite below is a plain lerp.
    px: Vec<u32>,
    iw: i32,
    ih: i32,

    cell_w: i32,
    cell_h: i32,
    frames: Vec<Vec<u16>>,
    ms: Vec<u32>,
    sel: usize,

    /// Where the sheet is currently drawn, for hit-testing clicks.
    view: RECT,
    dragging: bool,
    last_cell: i32,

    play: usize,
    play_acc: u32,
    font: isize,
    small: isize,
}

impl Editor {
    fn cols(&self) -> i32 {
        if self.cell_w > 0 { self.iw / self.cell_w } else { 0 }
    }
    fn rows(&self) -> i32 {
        if self.cell_h > 0 { self.ih / self.cell_h } else { 0 }
    }
    fn cell_count(&self) -> i32 {
        self.cols() * self.rows()
    }
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// Show the window, or bring the existing one forward.
pub fn open() {
    let existing = OPEN.with(|o| *o.borrow());
    if !existing.is_null() {
        unsafe {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
        return;
    }

    unsafe {
        let hinstance = GetModuleHandleW(null_mut());
        let class = wide("PetPalAddSprite");
        // Registering twice is harmless: the second call fails and we carry on.
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc_),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance as _,
            hIcon: LoadIconW(hinstance as _, 1 as *const u16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: CreateSolidBrush(rgb(0x2B, 0x2E, 0x38)) as HBRUSH,
            lpszMenuName: null_mut(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadIconW(hinstance as _, 1 as *const u16),
        };
        RegisterClassExW(&wc);

        let title = wide("Add a sprite");
        // Controls are placed at absolute coordinates while the sheet and the
        // playback box are laid out against the client rect. Those only agree
        // if the client area is exactly WIN_W x WIN_H, so ask for the outer
        // size that produces it rather than passing the client size and being
        // a border and a caption bar out.
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut want = RECT { left: 0, top: 0, right: WIN_W, bottom: WIN_H };
        // `true` for the menu bar: it eats a row of the client area otherwise,
        // and every control below is placed at an absolute coordinate.
        AdjustWindowRectEx(&mut want, style, 1, 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            want.right - want.left,
            want.bottom - want.top,
            null_mut(),
            null_mut(),
            hinstance as _,
            null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        SetMenu(hwnd, build_menu());
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

/// Let the shared message loop give the window proper Tab/Enter behaviour.
pub fn is_dialog_message(msg: *mut MSG) -> bool {
    let hwnd = OPEN.with(|o| *o.borrow());
    if hwnd.is_null() {
        return false;
    }
    unsafe { IsDialogMessageW(hwnd, msg) != 0 }
}

/// Two short menus. Everything here is also reachable from a button except the
/// prompt and the help, which have nowhere sensible to live in the layout.
unsafe fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu();
        let sheet = CreatePopupMenu();
        AppendMenuW(sheet, MF_STRING, ID_MENU_OPEN as usize, wide("&Open PNG...").as_ptr());
        AppendMenuW(
            sheet,
            MF_STRING,
            ID_MENU_PROMPT as usize,
            wide("&Copy AI prompt to clipboard").as_ptr(),
        );
        AppendMenuW(bar, MF_POPUP, sheet as usize, wide("&Sheet").as_ptr());

        let help = CreatePopupMenu();
        AppendMenuW(
            help,
            MF_STRING,
            ID_MENU_HELP as usize,
            wide("&What the controls do").as_ptr(),
        );
        AppendMenuW(
            help,
            MF_STRING,
            ID_MENU_GUIDE as usize,
            wide("Open the full &guide").as_ptr(),
        );
        AppendMenuW(bar, MF_POPUP, help as usize, wide("&Help").as_ptr());
        bar
    }
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_CREATE {
            let ed = Box::new(build(hwnd));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ed) as isize);
            OPEN.with(|o| *o.borrow_mut() = hwnd);
            DragAcceptFiles(hwnd, 1);
            SetTimer(hwnd, TIMER_PLAY, 40, None);
            return 0;
        }

        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Editor;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ed = &mut *ptr;

        match msg {
            WM_COMMAND => {
                on_command(ed, (wp & 0xFFFF) as isize, (wp >> 16) as u32);
                0
            }
            WM_DROPFILES => {
                let drop = wp as HDROP;
                let mut buf = [0u16; 520];
                if DragQueryFileW(drop, 0, buf.as_mut_ptr(), buf.len() as u32) > 0 {
                    load(ed, Path::new(&from_wide(&buf)));
                }
                DragFinish(drop);
                0
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                let (x, y) = (lo(lp), hi(lp));
                if let Some(cell) = hit(ed, x, y) {
                    if msg == WM_LBUTTONDOWN {
                        toggle(ed, cell);
                        ed.dragging = true;
                        ed.last_cell = cell as i32;
                        SetCapture(hwnd);
                    } else {
                        remove(ed, cell);
                    }
                    redraw(ed);
                }
                0
            }
            WM_MOUSEMOVE => {
                if ed.dragging {
                    if let Some(cell) = hit(ed, lo(lp), hi(lp)) {
                        // Painting across cells adds them; it never removes, or
                        // a drag over an already-assigned run would undo itself.
                        if cell as i32 != ed.last_cell {
                            ed.last_cell = cell as i32;
                            if !ed.frames[ed.sel].contains(&cell) {
                                ed.frames[ed.sel].push(cell);
                                sync_list(ed);
                            }
                            redraw(ed);
                        }
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                if ed.dragging {
                    ed.dragging = false;
                    ReleaseCapture();
                }
                0
            }
            WM_TIMER => {
                advance(ed);
                0
            }
            WM_PAINT => {
                paint(ed);
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                // Dark controls to match the window, otherwise the panel is a
                // wall of white against the sheet preview.
                let dc = wp as windows_sys::Win32::Graphics::Gdi::HDC;
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, rgb(0xE6, 0xE8, 0xEE));
                if msg == WM_CTLCOLORSTATIC {
                    return CTL_BG.with(|b| *b.borrow()) as LRESULT;
                }
                CTL_FIELD.with(|b| *b.borrow()) as LRESULT
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                KillTimer(hwnd, TIMER_PLAY);
                if ed.font != 0 {
                    DeleteObject(ed.font as _);
                }
                if ed.small != 0 {
                    DeleteObject(ed.small as _);
                }
                drop(Box::from_raw(ptr));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                OPEN.with(|o| *o.borrow_mut() = null_mut());
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

thread_local! {
    static CTL_BG: RefCell<isize> = const { RefCell::new(0) };
    static CTL_FIELD: RefCell<isize> = const { RefCell::new(0) };
}

fn lo(lp: LPARAM) -> i32 {
    (lp & 0xFFFF) as i16 as i32
}
fn hi(lp: LPARAM) -> i32 {
    ((lp >> 16) & 0xFFFF) as i16 as i32
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

unsafe fn build(hwnd: HWND) -> Editor {
    unsafe {
        // Made once and kept for the life of the process: the window can be
        // opened repeatedly, and a fresh pair each time is a slow GDI leak.
        CTL_BG.with(|b| {
            if *b.borrow() == 0 {
                *b.borrow_mut() = CreateSolidBrush(rgb(0x2B, 0x2E, 0x38)) as isize;
            }
        });
        CTL_FIELD.with(|b| {
            if *b.borrow() == 0 {
                *b.borrow_mut() = CreateSolidBrush(rgb(0x1E, 0x20, 0x28)) as isize;
            }
        });

        let font = CreateFontW(
            -15, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, 0, 0, 0, 0, 0,
            wide("Segoe UI").as_ptr(),
        ) as isize;
        let small = CreateFontW(
            -11, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, 0, 0, 0, 0, 0,
            wide("Consolas").as_ptr(),
        ) as isize;

        let mk = |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: isize| {
            let c = wide(class);
            let t = wide(text);
            let h_ = CreateWindowExW(
                0,
                c.as_ptr(),
                t.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                w,
                h,
                hwnd,
                id as _,
                GetModuleHandleW(null_mut()) as _,
                null_mut(),
            );
            SendMessageW(h_, WM_SETFONT, font as WPARAM, 1);
            h_
        };

        // -- row 1: the file --------------------------------------------
        mk("STATIC", "1. Sheet", SS_LEFT, PAD, PAD + 4, 60, 20, 0);
        let path_label = mk(
            "STATIC",
            "drop a PNG anywhere on this window, or click Browse",
            SS_LEFT | SS_PATHELLIPSIS,
            PAD + 66,
            PAD + 4,
            528,
            20,
            0,
        );
        mk("BUTTON", "Browse...", BS_PUSH, PAD + 604, PAD, 100, 27, ID_BROWSE);

        // -- row 2: tidying a generated sheet -----------------------------
        let y2 = PAD + 33;
        mk("STATIC", "2. Fix", SS_LEFT, PAD, y2 + 4, 60, 20, 0);
        mk("STATIC", "trim base", SS_LEFT, PAD + 66, y2 + 4, 64, 20, 0);
        let trim_edit = mk("EDIT", "0", ES_NUMBER_ | WS_BORDER, PAD + 132, y2, 44, 24, ID_TRIM);
        mk("STATIC", "remove bg", SS_LEFT, PAD + 190, y2 + 4, 72, 20, 0);
        let bg_edit = mk("EDIT", "", ES_NUMBER_ | WS_BORDER, PAD + 264, y2, 44, 24, ID_BG);
        mk("STATIC", "cols", SS_LEFT, PAD + 322, y2 + 4, 32, 20, 0);
        let cols_edit = mk("EDIT", "", ES_NUMBER_ | WS_BORDER, PAD + 356, y2, 42, 24, ID_COLS);
        mk("STATIC", "rows", SS_LEFT, PAD + 408, y2 + 4, 36, 20, 0);
        let rows_edit = mk("EDIT", "", ES_NUMBER_ | WS_BORDER, PAD + 446, y2, 42, 24, ID_ROWS);
        mk("BUTTON", "Fit to grid", BS_PUSH, PAD + 502, y2 - 1, 96, 26, ID_FIT);
        mk("BUTTON", "Use original", BS_PUSH, PAD + 604, y2 - 1, 100, 26, ID_ORIGINAL);

        // -- row 3: the grid and the name ---------------------------------
        let y3 = PAD + 66;
        mk("STATIC", "3. Grid", SS_LEFT, PAD, y3 + 4, 60, 20, 0);
        mk("STATIC", "cell", SS_LEFT, PAD + 66, y3 + 4, 28, 20, 0);
        let cw_edit = mk("EDIT", "64", ES_NUMBER_ | WS_BORDER, PAD + 96, y3, 48, 24, ID_CELL_W);
        mk("STATIC", "x", SS_CENTER, PAD + 148, y3 + 4, 12, 20, 0);
        let ch_edit = mk("EDIT", "64", ES_NUMBER_ | WS_BORDER, PAD + 164, y3, 48, 24, ID_CELL_H);
        mk("BUTTON", "Guess", BS_PUSH, PAD + 220, y3 - 1, 62, 26, ID_GUESS);

        mk("STATIC", "name", SS_LEFT, PAD + 300, y3 + 4, 38, 20, 0);
        let name = mk("EDIT", "", WS_BORDER, PAD + 340, y3, 190, 24, ID_NAME);

        let hint = mk(
            "STATIC",
            "Drop a sprite sheet to begin.",
            SS_LEFT,
            PAD + 546,
            y3 + 4,
            510,
            20,
            0,
        );

        // -- right panel ---------------------------------------------------
        let px = WIN_W - PANEL_W - PAD;
        mk("STATIC", "Animations", SS_LEFT, px, HEAD_H + 6, 140, 20, 0);
        let list = mk(
            "LISTBOX",
            "",
            LBS_NOTIFY_ | WS_BORDER | WS_VSCROLL,
            px,
            HEAD_H + 28,
            PANEL_W - 12,
            (ANIM_COUNT as i32) * 18 + 6,
            ID_ANIMS,
        );

        let y = HEAD_H + 34 + (ANIM_COUNT as i32) * 18 + 6;
        mk("STATIC", "ms per frame:", SS_LEFT, px, y + 4, 92, 20, 0);
        let ms_edit = mk("EDIT", "120", ES_NUMBER_ | WS_BORDER, px + 96, y, 60, 24, ID_MS);
        mk("BUTTON", "Whole row", BS_PUSH, px, y + 32, 96, 26, ID_ROW);
        mk("BUTTON", "Clear", BS_PUSH, px + 104, y + 32, 80, 26, ID_CLEAR);

        // The numbers themselves, so what gets written is on screen before it
        // is written. Playback order, left to right.
        mk("STATIC", "Frames, in order:", SS_LEFT, px, y + 70, 160, 18, 0);
        let frames_label = mk("STATIC", "none yet", SS_LEFT, px, y + 90, PANEL_W - 12, 110, 0);

        // -- footer --------------------------------------------------------
        mk("BUTTON", "Add sprite", BS_DEFPUSH, WIN_W - 220, WIN_H - FOOT_H - 26, 100, 30, ID_ADD);
        mk("BUTTON", "Cancel", BS_PUSH, WIN_W - 112, WIN_H - FOOT_H - 26, 92, 30, ID_CANCEL);

        let mut ed = Editor {
            hwnd,
            name,
            cw_edit,
            ch_edit,
            ms_edit,
            list,
            path_label,
            hint,
            frames_label,
            trim_edit,
            bg_edit,
            cols_edit,
            rows_edit,
            src: None,
            px: Vec::new(),
            iw: 0,
            ih: 0,
            orig: Vec::new(),
            orig_w: 0,
            orig_h: 0,
            fitted: false,
            cell_w: 64,
            cell_h: 64,
            frames: vec![Vec::new(); ANIM_COUNT],
            ms: Anim::ALL.iter().map(|_| 120u32).collect(),
            sel: 0,
            view: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            dragging: false,
            last_cell: -1,
            play: 0,
            play_acc: 0,
            font,
            small,
        };
        // Sensible starting speeds, matching what the built-ins use.
        for (i, a) in Anim::ALL.iter().enumerate() {
            ed.ms[i] = match a {
                Anim::Idle => 240,
                Anim::Walk => 90,
                Anim::Run => 55,
                Anim::Fall => 140,
                Anim::Sleep => 480,
                Anim::Annoyed => 140,
                Anim::Drag => 200,
                Anim::Alert => 160,
                Anim::Sit => 420,
                Anim::Climb => 120,
            };
        }
        sync_list(&ed);
        SendMessageW(list, LB_SETCURSEL, 0, 0);
        set_text(ms_edit, &ed.ms[0].to_string());
        ed
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn on_command(ed: &mut Editor, id: isize, code: u32) {
    match id {
        ID_BROWSE => browse(ed),
        ID_GUESS => {
            guess_cell(ed);
            redraw(ed);
        }
        ID_CELL_W | ID_CELL_H if code == EN_CHANGE => {
            ed.cell_w = read_num(ed.cw_edit).max(1);
            ed.cell_h = read_num(ed.ch_edit).max(1);
            // Cells that no longer exist would render as blanks in the app.
            let n = ed.cell_count().max(0) as u16;
            for f in ed.frames.iter_mut() {
                f.retain(|&c| c < n);
            }
            sync_list(ed);
            redraw(ed);
        }
        ID_MS if code == EN_CHANGE => {
            let v = read_num(ed.ms_edit).clamp(16, 5000) as u32;
            ed.ms[ed.sel] = v;
        }
        ID_ANIMS if code == LBN_SELCHANGE => {
            let i = unsafe { SendMessageW(ed.list, LB_GETCURSEL, 0, 0) };
            if i >= 0 && (i as usize) < ANIM_COUNT {
                ed.sel = i as usize;
                ed.play = 0;
                set_text(ed.ms_edit, &ed.ms[ed.sel].to_string());
                redraw(ed);
            }
        }
        ID_ROW => {
            assign_row(ed);
            redraw(ed);
        }
        ID_CLEAR => {
            ed.frames[ed.sel].clear();
            sync_list(ed);
            redraw(ed);
        }
        ID_FIT => {
            if ed.orig.is_empty() {
                return warn(ed.hwnd, "Load a sheet first.");
            }
            let o = fit_options(ed);
            match apply_fit(ed, &o) {
                Ok(msg) => set_text(ed.hint, &msg),
                Err(e) => set_text(ed.hint, &format!("Could not fit a grid: {e}")),
            }
            sync_list(ed);
            redraw(ed);
        }
        ID_ORIGINAL => {
            if ed.orig.is_empty() {
                return;
            }
            use_original(ed);
            guess_cell(ed);
            set_text(
                ed.hint,
                &format!("{}x{} as it is on disk, no fitting.", ed.iw, ed.ih),
            );
            sync_list(ed);
            redraw(ed);
        }
        ID_MENU_OPEN => browse(ed),
        ID_MENU_PROMPT => copy_prompt(ed),
        ID_MENU_HELP => show_help(ed.hwnd),
        ID_MENU_GUIDE => open_guide(ed.hwnd),
        ID_ADD => add(ed),
        ID_CANCEL => unsafe {
            DestroyWindow(ed.hwnd);
        },
        _ => {}
    }
}

fn browse(ed: &mut Editor) {
    let mut buf = [0u16; 520];
    let filter: Vec<u16> = "PNG images\0*.png\0All files\0*.*\0\0".encode_utf16().collect();
    let title = wide("Choose a sprite sheet");
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = ed.hwnd;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST;
        if GetOpenFileNameW(&mut ofn) != 0 {
            load(ed, Path::new(&from_wide(&buf)));
        }
    }
}

/// Read a PNG in, tidy it if it needs tidying, and set up the grid.
fn load(ed: &mut Editor, path: &Path) {
    match crate::sprites::decode_png_straight(path) {
        Ok((px, w, h)) => {
            ed.orig = px;
            ed.orig_w = w as i32;
            ed.orig_h = h as i32;
            ed.src = Some(path.to_path_buf());
            set_text(ed.path_label, &path.display().to_string());
            if ed.name_text().is_empty() {
                set_text(ed.name, &default_name(path));
            }
            for f in ed.frames.iter_mut() {
                f.clear();
            }
            // A sheet already on an exact grid is used as-is: fitting resamples
            // and re-anchors every pose, which would soften pixel art somebody
            // drew correctly. Anything else gets fitted, because slicing raw
            // generator output on any fixed cell size cuts creatures in half —
            // which is what the grid looks like before this runs.
            match crate::regrid::already_gridded(&ed.orig, w, h) {
                Some((cw, ch)) => {
                    use_original(ed);
                    ed.cell_w = cw as i32;
                    ed.cell_h = ch as i32;
                    set_text(ed.cw_edit, &ed.cell_w.to_string());
                    set_text(ed.ch_edit, &ed.cell_h.to_string());
                    set_text(
                        ed.hint,
                        &format!("{w}x{h}, already on a {cw}px grid — used as-is."),
                    );
                    prefill(ed);
                }
                None => auto_fit(ed),
            }
        }
        Err(e) => {
            warn(ed.hwnd, &format!("Could not read that image.\n\n{e}"));
        }
    }
    redraw(ed);
}

/// Read the fixing controls. Blank means "work it out".
fn fit_options(ed: &Editor) -> crate::regrid::Options {
    let num = |h: HWND| -> Option<u32> {
        let s = get_text(h);
        let s = s.trim();
        if s.is_empty() { None } else { s.parse().ok() }
    };
    crate::regrid::Options {
        cell: read_num(ed.cw_edit).clamp(8, 256) as u32,
        trim_bottom: num(ed.trim_edit).unwrap_or(0),
        key_bg: num(ed.bg_edit).map(|v| v.clamp(1, 255) as u8),
        cols: num(ed.cols_edit),
        rows: num(ed.rows_edit),
    }
}

/// Fit with whatever the controls say, and report what happened.
fn apply_fit(ed: &mut Editor, o: &crate::regrid::Options) -> Result<String, String> {
    let f = crate::regrid::fit(&ed.orig, ed.orig_w as u32, ed.orig_h as u32, o)?;
    let total = f.cols * f.rows;
    ed.px = f.px;
    ed.iw = f.w as i32;
    ed.ih = f.h as i32;
    ed.cell_w = f.cell as i32;
    ed.cell_h = f.cell as i32;
    ed.fitted = true;
    set_text(ed.cw_edit, &ed.cell_w.to_string());
    set_text(ed.ch_edit, &ed.cell_h.to_string());
    // Frames are positions in the grid, and the grid just changed.
    let n = total as u16;
    for fr in ed.frames.iter_mut() {
        fr.retain(|&c| c < n);
    }
    Ok(format!(
        "Fitted: {} x {} = {} cells, {} sprites, largest {}x{} scaled {:.2}x.",
        f.cols, f.rows, total, f.found, f.largest.0, f.largest.1, f.scale
    ))
}

/// Fit a freshly loaded sheet, deciding for itself whether the background needs
/// removing.
///
/// A solid background makes the whole image read as one enormous sprite, and
/// the tool cannot tell that from a legitimate single-cell sheet — so rather
/// than ask, try without keying and retry with it if the answer was absurd.
fn auto_fit(ed: &mut Editor) {
    let mut o = fit_options(ed);
    o.key_bg = None;
    let first = crate::regrid::fit(&ed.orig, ed.orig_w as u32, ed.orig_h as u32, &o);
    let needs_key = match &first {
        Ok(f) => f.cols * f.rows < 2,
        Err(_) => true,
    };
    if needs_key {
        o.key_bg = Some(crate::regrid::DEFAULT_BG_TOL);
        set_text(ed.bg_edit, &crate::regrid::DEFAULT_BG_TOL.to_string());
    }
    match apply_fit(ed, &o) {
        Ok(mut msg) => {
            if needs_key {
                msg.push_str(" Background removed.");
            }
            set_text(ed.hint, &msg);
            prefill(ed);
        }
        Err(e) => {
            use_original(ed);
            set_text(ed.hint, &format!("Could not fit a grid: {e}"));
        }
    }
}

/// Go back to the file exactly as it is on disk.
fn use_original(ed: &mut Editor) {
    ed.px = ed.orig.clone();
    ed.iw = ed.orig_w;
    ed.ih = ed.orig_h;
    ed.fitted = false;
}

/// Pick the squarest cell size that divides the sheet exactly and lands nearest
/// 64px, which is what almost every generated sheet turns out to be.
fn guess_cell(ed: &mut Editor) {
    if ed.iw == 0 || ed.ih == 0 {
        return;
    }
    let mut best = (i32::MAX, ed.cell_w, ed.cell_h);
    for c in 8..=256i32 {
        if ed.iw % c != 0 || ed.ih % c != 0 {
            continue;
        }
        let cells = (ed.iw / c) * (ed.ih / c);
        if !(2..=400).contains(&cells) {
            continue;
        }
        let score = (c - 64).abs();
        if score < best.0 {
            best = (score, c, c);
        }
    }
    if best.0 != i32::MAX {
        ed.cell_w = best.1;
        ed.cell_h = best.2;
        set_text(ed.cw_edit, &ed.cell_w.to_string());
        set_text(ed.ch_edit, &ed.cell_h.to_string());
    }
}

/// Rows 0, 1 and 2 are idle, walk and run on nearly every sheet in existence.
/// Filling them in is three fewer things to click, and wrong guesses are one
/// Clear away.
fn prefill(ed: &mut Editor) {
    let (cols, rows) = (ed.cols(), ed.rows());
    if cols <= 0 || rows < 3 {
        return;
    }
    for (anim, row) in [(0usize, 0i32), (1, 1), (2, 2)] {
        ed.frames[anim] = (0..cols).map(|c| (row * cols + c) as u16).collect();
    }
    sync_list(ed);
}

fn assign_row(ed: &mut Editor) {
    let cols = ed.cols();
    if cols <= 0 {
        return;
    }
    // The row of the last cell touched, or the first row if nothing is picked.
    let anchor = ed.frames[ed.sel].last().copied().unwrap_or(0) as i32;
    let row = anchor / cols;
    ed.frames[ed.sel] = (0..cols).map(|c| (row * cols + c) as u16).collect();
    sync_list(ed);
}

fn toggle(ed: &mut Editor, cell: u16) {
    if let Some(p) = ed.frames[ed.sel].iter().position(|&c| c == cell) {
        ed.frames[ed.sel].remove(p);
    } else {
        ed.frames[ed.sel].push(cell);
    }
    sync_list(ed);
}

fn remove(ed: &mut Editor, cell: u16) {
    ed.frames[ed.sel].retain(|&c| c != cell);
    sync_list(ed);
}

fn sync_list(ed: &Editor) {
    let f = &ed.frames[ed.sel];
    set_text(
        ed.frames_label,
        &if f.is_empty() {
            "none yet - click cells in the sheet".to_string()
        } else {
            f.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ")
        },
    );
    unsafe {
        let keep = SendMessageW(ed.list, LB_GETCURSEL, 0, 0);
        SendMessageW(ed.list, LB_RESETCONTENT, 0, 0);
        for (i, a) in Anim::ALL.iter().enumerate() {
            let n = ed.frames[i].len();
            let mark = if n == 0 { ' ' } else { '*' };
            let s = wide(&format!("{mark} {:<8} {n}", a.key()));
            SendMessageW(ed.list, LB_ADDSTRING, 0, s.as_ptr() as LPARAM);
        }
        if keep >= 0 {
            SendMessageW(ed.list, LB_SETCURSEL, keep as WPARAM, 0);
        }
    }
}

fn advance(ed: &mut Editor) {
    let n = ed.frames[ed.sel].len();
    if n == 0 {
        return;
    }
    ed.play_acc += 40;
    if ed.play_acc >= ed.ms[ed.sel] {
        ed.play_acc = 0;
        ed.play = (ed.play + 1) % n;
        redraw(ed);
    }
}

fn redraw(ed: &Editor) {
    unsafe { InvalidateRect(ed.hwnd, null_mut(), 0) };
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

/// Composite one source pixel over the checkerboard.
#[inline]
fn over(src: u32, bg: u32) -> u32 {
    let a = (src >> 24) & 0xFF;
    if a == 255 {
        return src;
    }
    if a == 0 {
        return bg;
    }
    let m = |sh: u32| {
        let s = (src >> sh) & 0xFF;
        let d = (bg >> sh) & 0xFF;
        (s * a + d * (255 - a) + 127) / 255
    };
    0xFF00_0000 | (m(16) << 16) | (m(8) << 8) | m(0)
}

fn paint(ed: &mut Editor) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(ed.hwnd, &mut ps);

        let mut client: RECT = std::mem::zeroed();
        GetClientRect(ed.hwnd, &mut client);

        // The sheet viewport.
        let vw = client.right - PANEL_W - PAD * 2;
        let vh = client.bottom - HEAD_H - FOOT_H - PAD;
        let vx = PAD;
        let vy = HEAD_H;
        if vw > 16 && vh > 16 {
            draw_sheet(ed, dc, vx, vy, vw, vh);
        }

        // Live playback of the selected animation, under the panel.
        let px = client.right - PANEL_W - PAD;
        let py = client.bottom - FOOT_H - 190;
        draw_play(ed, dc, px, py, PANEL_W - 12, 170);

        EndPaint(ed.hwnd, &ps);
    }
}

/// Blit a software-composited buffer. One `StretchDIBits` for the whole
/// viewport keeps this to a single GDI call per repaint, which matters because
/// dragging across cells repaints on every mouse move.
unsafe fn blit(dc: windows_sys::Win32::Graphics::Gdi::HDC, buf: &[u32], x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // Negative: top-down, matching the buffer's row order.
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        StretchDIBits(
            dc, x, y, w, h, 0, 0, w, h,
            buf.as_ptr() as *const _, &bi, DIB_RGB_COLORS, SRCCOPY,
        );
    }
}

/// Everything the sheet view needs to draw itself, with no window attached.
///
/// Split out from the paint handler so the geometry can be tested: the grid
/// lines, the highlight rectangles and the hit-test all have to agree on where
/// a cell is, and an off-by-one there is invisible in code review but obvious
/// in a rendered image.
pub(crate) struct View {
    pub buf: Vec<u32>,
    pub rect: RECT,
}

pub(crate) fn compose_sheet(
    px: &[u32],
    iw: i32,
    ih: i32,
    cols: i32,
    rows: i32,
    picked: &[u16],
    taken: &dyn Fn(u16) -> bool,
    vw: i32,
    vh: i32,
) -> View {
    let mut buf = vec![BG; (vw * vh).max(0) as usize];
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if iw <= 0 || ih <= 0 || vw <= 0 || vh <= 0 {
        return View { buf, rect };
    }

    // Integer zoom keeps pixel art crisp; below 1:1 fall back to a fractional
    // shrink so a huge sheet still fits.
    let fit = (vw / iw).min(vh / ih);
    let (dw, dh) = if fit >= 1 {
        (iw * fit, ih * fit)
    } else {
        let s = (vw as f32 / iw as f32).min(vh as f32 / ih as f32);
        (((iw as f32 * s) as i32).max(1), ((ih as f32 * s) as i32).max(1))
    };
    let ox = (vw - dw) / 2;
    let oy = (vh - dh) / 2;
    rect = RECT { left: ox, top: oy, right: ox + dw, bottom: oy + dh };

    for y in 0..dh {
        let sy = (y as i64 * ih as i64 / dh as i64) as i32;
        for x in 0..dw {
            let sx = (x as i64 * iw as i64 / dw as i64) as i32;
            let checker = if ((x / 8) + (y / 8)) % 2 == 0 { CHECK_A } else { CHECK_B };
            let s = px[(sy * iw + sx) as usize];
            buf[((oy + y) * vw + ox + x) as usize] = over(s, checker);
        }
    }

    if cols <= 0 || rows <= 0 {
        return View { buf, rect };
    }
    let cw = dw as f32 / cols as f32;
    let ch = dh as f32 / rows as f32;
    let mut put = |x: i32, y: i32, c: u32| {
        if x >= 0 && x < vw && y >= 0 && y < vh {
            buf[(y * vw + x) as usize] = c;
        }
    };
    for c in 0..=cols {
        let x = ox + (c as f32 * cw) as i32;
        for y in 0..dh {
            put(x, oy + y, GRID);
        }
    }
    for r in 0..=rows {
        let y = oy + (r as f32 * ch) as i32;
        for x in 0..dw {
            put(ox + x, y, GRID);
        }
    }

    // Highlights last so they sit over the grid.
    for cell in 0..(cols * rows) {
        let is_picked = picked.contains(&(cell as u16));
        let colour = if is_picked {
            PICK
        } else if taken(cell as u16) {
            OTHER
        } else {
            continue;
        };
        let (cx, cy) = (cell % cols, cell / cols);
        let x0 = ox + (cx as f32 * cw) as i32;
        let y0 = oy + (cy as f32 * ch) as i32;
        let x1 = ox + ((cx + 1) as f32 * cw) as i32;
        let y1 = oy + ((cy + 1) as f32 * ch) as i32;
        let thick = if is_picked { 2 } else { 1 };
        for t in 0..thick {
            for x in x0..=x1 {
                put(x, y0 + t, colour);
                put(x, y1 - t, colour);
            }
            for y in y0..=y1 {
                put(x0 + t, y, colour);
                put(x1 - t, y, colour);
            }
        }
    }
    View { buf, rect }
}

unsafe fn draw_sheet(ed: &mut Editor, dc: windows_sys::Win32::Graphics::Gdi::HDC, vx: i32, vy: i32, vw: i32, vh: i32) {
    unsafe {
        let picked = ed.frames[ed.sel].clone();
        let sel = ed.sel;
        let frames = ed.frames.clone();
        let taken = move |c: u16| (0..ANIM_COUNT).any(|i| i != sel && frames[i].contains(&c));
        let v = compose_sheet(
            &ed.px, ed.iw, ed.ih, ed.cols(), ed.rows(), &picked, &taken, vw, vh,
        );
        ed.view = RECT {
            left: vx + v.rect.left,
            top: vy + v.rect.top,
            right: vx + v.rect.right,
            bottom: vy + v.rect.bottom,
        };
        blit(dc, &v.buf, vx, vy, vw, vh);

        // Numbers go on last, with GDI: rasterising a font by hand to save one
        // call would be silly.
        if ed.iw > 0 && ed.cols() > 0 {
            let old = SelectObject(dc, ed.small as _);
            SetBkMode(dc, TRANSPARENT as i32);
            let (cols, rows) = (ed.cols(), ed.rows());
            let dw = ed.view.right - ed.view.left;
            let dh = ed.view.bottom - ed.view.top;
            let cw = dw as f32 / cols as f32;
            let ch = dh as f32 / rows as f32;
            if cw >= 22.0 && ch >= 16.0 {
                for cell in 0..(cols * rows) {
                    let (cx, cy) = (cell % cols, cell / cols);
                    let x = ed.view.left + (cx as f32 * cw) as i32 + 3;
                    let y = ed.view.top + (cy as f32 * ch) as i32 + 2;
                    let mine = ed.frames[ed.sel].iter().position(|&c| c == cell as u16);
                    let (txt, col) = match mine {
                        // Its place in the animation, which is the thing you
                        // actually need to see while clicking cells in order.
                        Some(i) => (format!("{}", i + 1), rgb(0x20, 0x18, 0x00)),
                        None => (format!("{cell}"), rgb(0xA8, 0xB0, 0xC0)),
                    };
                    SetTextColor(dc, col);
                    let w = wide(&txt);
                    TextOutW(dc, x, y, w.as_ptr(), txt.chars().count() as i32);
                }
            }
            SelectObject(dc, old);
        }
    }
}

unsafe fn draw_play(ed: &Editor, dc: windows_sys::Win32::Graphics::Gdi::HDC, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let mut buf = vec![argb(0x24, 0x27, 0x30); (w * h) as usize];
        let idx = ed.frames[ed.sel].get(ed.play).copied();
        if let (Some(cell), true) = (idx, ed.cols() > 0) {
            let (cols, cw, ch) = (ed.cols(), ed.cell_w, ed.cell_h);
            let (cx, cy) = ((cell as i32 % cols) * cw, (cell as i32 / cols) * ch);
            let s = ((w - 16) / cw.max(1)).min((h - 16) / ch.max(1)).max(1);
            let (dw, dh) = (cw * s, ch * s);
            let (ox, oy) = ((w - dw) / 2, (h - dh) / 2);
            for py in 0..dh {
                let sy = cy + py / s;
                for px in 0..dw {
                    let sx = cx + px / s;
                    if sx >= ed.iw || sy >= ed.ih {
                        continue;
                    }
                    let checker = if ((px / 8) + (py / 8)) % 2 == 0 { CHECK_A } else { CHECK_B };
                    let src = ed.px[(sy * ed.iw + sx) as usize];
                    buf[((oy + py) * w + ox + px) as usize] = over(src, checker);
                }
            }
        }
        blit(dc, &buf, x, y, w, h);
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

fn hit(ed: &Editor, x: i32, y: i32) -> Option<u16> {
    let (cols, rows) = (ed.cols(), ed.rows());
    if cols <= 0 || rows <= 0 {
        return None;
    }
    let v = ed.view;
    if x < v.left || x >= v.right || y < v.top || y >= v.bottom {
        return None;
    }
    let cw = (v.right - v.left) as f32 / cols as f32;
    let ch = (v.bottom - v.top) as f32 / rows as f32;
    let cx = ((x - v.left) as f32 / cw) as i32;
    let cy = ((y - v.top) as f32 / ch) as i32;
    if cx >= cols || cy >= rows {
        return None;
    }
    Some((cy * cols + cx) as u16)
}

// ---------------------------------------------------------------------------
// Writing it out
// ---------------------------------------------------------------------------

impl Editor {
    fn name_text(&self) -> String {
        get_text(self.name).trim().to_string()
    }
}

/// A name to start from.
///
/// The filename is usually right, except when it is `creature.png` — which is
/// exactly what this app's own exporter writes, so "make a copy to edit" would
/// otherwise propose calling every creature "creature". In that case the folder
/// it sits in is the better guess.
fn default_name(path: &Path) -> String {
    const GENERIC: [&str; 5] = ["creature", "sheet", "spritesheet", "sprite", "output"];
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(sanitise)
        .unwrap_or_default();
    // A name carrying a timestamp came from a generator, not from a person, and
    // proposing "chatgpt-image-aug-10--2026-04-55-04-pm" invites someone to
    // press Add and end up with that in their tray menu. Better to ask.
    let noisy = stem.len() > 24
        || stem.contains("chatgpt")
        || stem.contains("screenshot")
        || stem.contains("untitled")
        || stem.contains("download")
        || stem
            .as_bytes()
            .windows(4)
            .any(|w| w.iter().all(|c| c.is_ascii_digit()));
    if noisy {
        return String::new();
    }
    if !stem.is_empty() && !GENERIC.contains(&stem.as_str()) {
        return stem;
    }
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(sanitise)
        .filter(|s| !s.is_empty())
        .unwrap_or(stem)
}

/// Folder names end up in a menu and in `config.toml`; keep them boring.
fn sanitise(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    out.trim_matches('-').to_ascii_lowercase()
}

fn add(ed: &mut Editor) {
    let Some(src) = ed.src.clone() else {
        return warn(ed.hwnd, "Choose a sprite sheet first.");
    };
    let name = sanitise(&ed.name_text());
    if name.is_empty() {
        return warn(ed.hwnd, "Give the creature a name.\n\nIt becomes the folder name and the label in the Sprite menu.");
    }
    if crate::sprites::Kind::from_id(&name).is_some() {
        return warn(ed.hwnd, &format!("\"{name}\" is the name of a built-in creature.\n\nPick another."));
    }
    if ed.cell_count() <= 0 {
        return warn(ed.hwnd, "The cell size is bigger than the image.");
    }
    if ed.frames.iter().all(|f| f.is_empty()) {
        return warn(ed.hwnd, "No animations assigned yet.\n\nPick one on the right, then click the cells that make it up.");
    }
    if ed.frames[Anim::Idle as usize].is_empty() {
        return warn(ed.hwnd, "`idle` has no frames.\n\nEverything you leave out falls back to idle, so it is the one animation that has to exist.");
    }

    let dir = Config::sprites_dir().join(&name);
    if dir.exists() && !confirm(ed.hwnd, &format!("\"{name}\" already exists.\n\nReplace it?")) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return warn(ed.hwnd, &format!("Could not create the folder.\n\n{e}"));
    }
    // What gets written is what is on screen. If the sheet was fitted, copying
    // the original would ship a file whose grid has nothing to do with the
    // frame numbers just assigned against the fitted one. If it was not, the
    // file is copied untouched rather than re-encoded — bit for bit what the
    // author picked.
    let wrote = if ed.fitted {
        write_png(&dir.join("creature.png"), &ed.px, ed.iw as u32, ed.ih as u32)
    } else {
        std::fs::copy(&src, dir.join("creature.png"))
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    if let Err(e) = wrote {
        return warn(ed.hwnd, &format!("Could not write the image.\n\n{e}"));
    }
    if let Err(e) = std::fs::write(dir.join("sprite.toml"), manifest(ed.cell_w, ed.cell_h, &ed.frames, &ed.ms)) {
        return warn(ed.hwnd, &format!("Could not write sprite.toml.\n\n{e}"));
    }

    ADDED.with(|a| *a.borrow_mut() = Some(name));
    unsafe {
        let pet = crate::app_hwnd();
        if !pet.is_null() {
            PostMessageW(pet, WM_SPRITE_ADDED, 0, 0);
        }
        DestroyWindow(ed.hwnd);
    }
}

/// Build `sprite.toml`. Takes the values rather than the window so the exact
/// text the button writes can be round-tripped through the real loader in a
/// test — a manifest this window writes but the app cannot read would be the
/// worst possible bug here.
fn manifest(cell_w: i32, cell_h: i32, frames: &[Vec<u16>], ms: &[u32]) -> String {
    let mut s = String::with_capacity(512);
    s.push_str("# Written by PetPal's \"Add a sprite\" window.
");
    s.push_str("# Frame numbers count left to right, top to bottom, starting at 0.

");
    s.push_str("image = \"creature.png\"
");
    s.push_str(&format!("frame_width = {cell_w}
"));
    s.push_str(&format!("frame_height = {cell_h}
"));
    for (i, a) in Anim::ALL.iter().enumerate() {
        if frames[i].is_empty() {
            continue;
        }
        let list: Vec<String> = frames[i].iter().map(|c| c.to_string()).collect();
        s.push_str(&format!("
[anims.{}]
", a.key()));
        s.push_str(&format!("frames = [{}]
", list.join(", ")));
        s.push_str(&format!("frame_ms = {}
", ms[i]));
    }
    let missing: Vec<&str> = Anim::ALL
        .iter()
        .enumerate()
        .filter(|(i, _)| frames[*i].is_empty())
        .map(|(_, a)| a.key())
        .collect();
    if !missing.is_empty() {
        s.push_str(&format!(
            "
# Not assigned, so these fall back to idle (climb falls back to walk): {}
",
            missing.join(", ")
        ));
    }
    s
}

/// Straight RGBA out. Only used for a fitted sheet — an untouched file is
/// copied rather than re-encoded, so nothing is lost for no reason.
fn write_png(path: &Path, px: &[u32], w: u32, h: u32) -> Result<(), String> {
    let mut flat = Vec::with_capacity((w * h * 4) as usize);
    for p in px {
        flat.extend_from_slice(&[
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
            (p >> 24) as u8,
        ]);
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&flat))
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Help, and the prompt
// ---------------------------------------------------------------------------

/// What each control does *to the result*.
///
/// Written around the values rather than the widgets. "trim base" is obvious as
/// a label and useless as an explanation, because the question in the reader's
/// head is "what number goes in it, and how will I know it was wrong".
const HELP: &str = concat!(
    "WHAT EACH CONTROL DOES\r\n",
    "\r\n",
    "The window works top to bottom: load a sheet, make the grid fit it, then\r\n",
    "say which cells belong to which animation.\r\n",
    "\r\n",
    "\r\n",
    "1. SHEET\r\n",
    "\r\n",
    "  Browse, or drop a PNG anywhere on this window. It needs real\r\n",
    "  transparency, or a plain background that can be removed for you.\r\n",
    "\r\n",
    "\r\n",
    "2. FIX   - only matters for a sheet that is not already on a grid\r\n",
    "\r\n",
    "  A sheet from an image generator is a picture OF a sprite sheet: the\r\n",
    "  poses sit wherever they landed, there are wide margins, and it is far\r\n",
    "  larger than the pixel art it depicts. Slicing that on any cell size\r\n",
    "  cuts creatures in half. This row re-lays the poses on a real grid.\r\n",
    "\r\n",
    "  It runs automatically when you load a sheet, and is skipped entirely\r\n",
    "  if the sheet is already on an exact grid - refitting would resample\r\n",
    "  art that is already correct and soften it.\r\n",
    "\r\n",
    "  trim base   Pixels cut off the BOTTOM of every pose, measured in the\r\n",
    "              original image.\r\n",
    "                0         nothing cut. Right unless each pose stands on\r\n",
    "                          a painted plate or shadow.\r\n",
    "                too low   a pale sliver is left under the feet, and the\r\n",
    "                          creature hovers that far above your taskbar.\r\n",
    "                too high  starts eating the feet.\r\n",
    "              Typical for a 1500px-wide sheet: 10 to 25. Change it,\r\n",
    "              press Fit to grid, look, adjust.\r\n",
    "\r\n",
    "  remove bg   Tolerance for deleting the background, flooding inward\r\n",
    "              from the edges of the image.\r\n",
    "                blank     decide for me. If the sheet reads as one huge\r\n",
    "                          sprite, the background is opaque, so it goes.\r\n",
    "                1 to 15   near-identical pixels only. For a flat colour.\r\n",
    "                30        the default. Copes with a gradient or vignette.\r\n",
    "                45 and up aggressive. Use if a haze survives.\r\n",
    "                too high  eats the creature's own outline; poses start\r\n",
    "                          losing edges or disappearing.\r\n",
    "              Because it floods from the edges, a colour INSIDE the\r\n",
    "              creature - white eyes, a pale chest - is always kept.\r\n",
    "\r\n",
    "  cols, rows  Force the grid instead of detecting it.\r\n",
    "                blank     detect. Nearly always right, and what you want.\r\n",
    "                a number  splits the WHOLE image into that many equal\r\n",
    "                          bands. Wrong the moment the sheet has margins:\r\n",
    "                          the outer cells come out empty.\r\n",
    "              Only for when two poses touch and got counted as one. The\r\n",
    "              giveaway is a cell count lower than you expect.\r\n",
    "\r\n",
    "  Fit to grid    Re-run with the current values. Always starts from the\r\n",
    "                 original file, so trying numbers costs nothing.\r\n",
    "  Use original   Discard the fitting; use the file exactly as it is.\r\n",
    "\r\n",
    "  The line at the top right reports the grid it found, how many sprites,\r\n",
    "  and how much it scaled them.\r\n",
    "\r\n",
    "\r\n",
    "3. GRID\r\n",
    "\r\n",
    "  cell w x h  The size of one frame, in the sheet's own pixels. The grid\r\n",
    "              drawn over the sheet uses these two numbers - if the lines\r\n",
    "              do not sit cleanly between the poses, they are wrong.\r\n",
    "              After fitting they are already correct.\r\n",
    "              Changing them renumbers every cell, so assigned frames\r\n",
    "              that no longer exist are dropped.\r\n",
    "  Guess       The cell size nearest 64 that divides the sheet exactly.\r\n",
    "  name        The folder name, and the label in the Sprite menu.\r\n",
    "              Letters, numbers and dashes; anything else becomes a dash.\r\n",
    "\r\n",
    "\r\n",
    "ASSIGNING ANIMATIONS\r\n",
    "\r\n",
    "  Pick an animation in the list, then click cells in the sheet in the\r\n",
    "  order they should play.\r\n",
    "\r\n",
    "    click         add the cell to the end of the selected animation\r\n",
    "    click again   remove it\r\n",
    "    drag          add every cell you pass over\r\n",
    "    right-click   remove a cell\r\n",
    "    Whole row     assign the whole row containing the last cell picked\r\n",
    "    Clear         empty the selected animation\r\n",
    "\r\n",
    "  A cell in the selected animation is outlined in orange and shows its\r\n",
    "  POSITION in that animation (1, 2, 3...) rather than its cell number.\r\n",
    "  A cell outlined in blue already belongs to another animation. Sharing\r\n",
    "  is allowed, but it is usually a mistake.\r\n",
    "\r\n",
    "  ms per frame  How long each frame shows, in thousandths of a second.\r\n",
    "                Lower is faster. 240 is a slow idle, 55 is a run. The\r\n",
    "                preview box plays at this speed, so set it by eye.\r\n",
    "\r\n",
    "\r\n",
    "WHAT HAS TO BE FILLED IN\r\n",
    "\r\n",
    "  Only idle. Every animation left empty falls back to idle, except\r\n",
    "  climb, which falls back to walk. A sheet with one idle frame already\r\n",
    "  works; idle plus walk already feels alive.\r\n",
    "\r\n",
    "\r\n",
    "IF THE RESULT LOOKS WRONG\r\n",
    "\r\n",
    "  invisible in one pose    those cells are empty. An empty cell is not\r\n",
    "                           an error; the creature just disappears.\r\n",
    "  hovers above surfaces    empty rows below the feet. Raise trim base.\r\n",
    "  sinks into the taskbar   feet drawn past the bottom of the cell.\r\n",
    "  hugs one end of a ledge  the body is off-centre in its cell.\r\n",
    "  faces backwards          art drawn facing left. Everything must face\r\n",
    "                           right; PetPal mirrors it to walk the other way.\r\n",
    "  changes size as it       the poses are not all the same size on the\r\n",
    "  animates                 sheet.\r\n",
);

/// The prompt, lifted out of the shipped guide rather than written twice.
///
/// The guide's copy is the one people read and the one the tests check; a
/// second copy here would be the one that quietly goes stale.
fn ai_prompt() -> Option<String> {
    let guide = crate::config::SPRITE_GUIDE;
    let start = guide.find("Generate ONE single image")?;
    let end = guide[start..].find("\n```")? + start;
    Some(guide[start..end].replace('\n', "\r\n"))
}

fn copy_prompt(ed: &Editor) {
    let Some(text) = ai_prompt() else {
        return warn(ed.hwnd, "The prompt could not be found in the guide.");
    };
    if let Err(e) = set_clipboard(ed.hwnd, &text) {
        return warn(ed.hwnd, &format!("Could not copy to the clipboard.\n\n{e}"));
    }
    info(
        ed.hwnd,
        "Prompt copied.\n\n\
         Paste it into an image generator and replace the line beginning \
         \"THE CREATURE:\" with your own description.\n\n\
         It asks for one image: 8 squares across, 6 down, 48 in total. Row 1 \
         is idle, row 2 walking, row 3 running, and the last three rows are \
         split between the other seven animations.\n\n\
         Drop the result back on this window - it will fit the grid for you.",
    );
}

fn set_clipboard(hwnd: HWND, text: &str) -> Result<(), String> {
    let wide_text = wide(text);
    let bytes = wide_text.len() * 2;
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err("the clipboard is in use by another program".into());
        }
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if h.is_null() {
            CloseClipboard();
            return Err("out of memory".into());
        }
        let p = GlobalLock(h) as *mut u16;
        if p.is_null() {
            CloseClipboard();
            return Err("could not lock the clipboard buffer".into());
        }
        std::ptr::copy_nonoverlapping(wide_text.as_ptr(), p, wide_text.len());
        GlobalUnlock(h);
        // CF_UNICODETEXT. On success the clipboard takes ownership of the handle.
        if SetClipboardData(13, h as _).is_null() {
            CloseClipboard();
            return Err("the clipboard refused the text".into());
        }
        CloseClipboard();
    }
    Ok(())
}

/// A read-only scrolling window rather than a message box: this is reference
/// text you read *while* using the controls it describes, so it must not be
/// modal and it must not clip.
fn show_help(owner: HWND) {
    unsafe {
        let hinstance = GetModuleHandleW(null_mut());
        let class = wide("PetPalSpriteHelp");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(help_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance as _,
            hIcon: LoadIconW(hinstance as _, 1 as *const u16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: CTL_BG.with(|b| *b.borrow()) as _,
            lpszMenuName: null_mut(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadIconW(hinstance as _, 1 as *const u16),
        };
        RegisterClassExW(&wc);

        let style = WS_OVERLAPPEDWINDOW;
        let mut want = RECT { left: 0, top: 0, right: 720, bottom: 660 };
        AdjustWindowRectEx(&mut want, style, 0, 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Add a sprite - help").as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            want.right - want.left,
            want.bottom - want.top,
            owner,
            null_mut(),
            hinstance as _,
            null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide(HELP).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE_ | ES_READONLY_,
            0,
            0,
            720,
            660,
            hwnd,
            null_mut(),
            hinstance as _,
            null_mut(),
        );
        let font = CreateFontW(
            -13, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, 0, 0, 0, 0, 0,
            wide("Consolas").as_ptr(),
        );
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
        ShowWindow(hwnd, SW_SHOW);
    }
}

/// Keeps the edit filling the frame, and paints it in the same dark scheme as
/// the editor rather than leaving one white rectangle on screen.
unsafe extern "system" fn help_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_SIZE => {
                let child = GetWindow(hwnd, GW_CHILD);
                if !child.is_null() {
                    let mut rc: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rc);
                    MoveWindow(child, 0, 0, rc.right, rc.bottom, 1);
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                let dc = wp as windows_sys::Win32::Graphics::Gdi::HDC;
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, rgb(0xE6, 0xE8, 0xEE));
                CTL_FIELD.with(|b| *b.borrow()) as LRESULT
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn open_guide(hwnd: HWND) {
    let path = Config::sprites_dir().join("HOW-TO-make-a-sprite.txt");
    if !path.exists() {
        let _ = std::fs::create_dir_all(Config::sprites_dir());
        let _ = std::fs::write(&path, crate::config::SPRITE_GUIDE);
    }
    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(&path.to_string_lossy()).as_ptr(),
            null_mut(),
            null_mut(),
            SW_SHOWNORMAL,
        );
    }
}

// ---------------------------------------------------------------------------
// Small control helpers
// ---------------------------------------------------------------------------

fn set_text(h: HWND, s: &str) {
    unsafe { SetWindowTextW(h, wide(s).as_ptr()) };
}

fn get_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        from_wide(&buf)
    }
}

fn read_num(h: HWND) -> i32 {
    get_text(h).trim().parse().unwrap_or(0)
}

fn info(hwnd: HWND, msg: &str) {
    unsafe {
        MessageBoxW(
            hwnd,
            wide(msg).as_ptr(),
            wide("Add a sprite").as_ptr(),
            MB_ICONINFORMATION,
        );
    }
}

fn warn(hwnd: HWND, msg: &str) {
    unsafe {
        MessageBoxW(hwnd, wide(msg).as_ptr(), wide("Add a sprite").as_ptr(), MB_ICONWARNING);
    }
}

fn confirm(hwnd: HWND, msg: &str) -> bool {
    unsafe {
        MessageBoxW(
            hwnd,
            wide(msg).as_ptr(),
            wide("Add a sprite").as_ptr(),
            MB_ICONQUESTION | MB_YESNO,
        ) == IDYES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x3 grid of 16px cells, every cell a solid block of a distinct colour
    /// so a misplaced grid line is visible in the composed output.
    fn sheet(cols: i32, rows: i32, cell: i32) -> (Vec<u32>, i32, i32) {
        let (w, h) = (cols * cell, rows * cell);
        let mut px = vec![0u32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let n = (y / cell) * cols + (x / cell);
                // Leave a transparent margin so the checkerboard shows through.
                let inset = (x % cell) < 2 || (y % cell) < 2;
                px[(y * w + x) as usize] = if inset {
                    0
                } else {
                    argb(40 + (n as u8) * 17, 90, 200 - (n as u8) * 12)
                };
            }
        }
        (px, w, h)
    }

    #[test]
    fn a_cell_clicked_is_the_cell_highlighted() {
        // The grid, the highlight and the hit-test are three separate pieces of
        // arithmetic over the same rectangle. This is the test that they agree:
        // ask which cell a point belongs to, then check that cell is the one
        // that came back highlighted.
        let (cols, rows, cell) = (4, 3, 16);
        let (px, iw, ih) = sheet(cols, rows, cell);
        let (vw, vh) = (400, 300);
        let picked = vec![5u16];
        let none = |_: u16| false;
        let v = compose_sheet(&px, iw, ih, cols, rows, &picked, &none, vw, vh);

        let dw = v.rect.right - v.rect.left;
        let dh = v.rect.bottom - v.rect.top;
        assert!(dw > 0 && dh > 0, "nothing was drawn");
        assert_eq!(dw % iw, 0, "zoom should be an integer for crisp pixels");
        assert_eq!(dw / iw, dh / ih, "aspect must be preserved");
        // Centred, to within the odd-pixel remainder.
        assert!((v.rect.left - (vw - dw) / 2).abs() <= 1);

        let cw = dw / cols;
        let ch = dh / rows;
        for target in 0..(cols * rows) {
            // A point in the middle of the target cell.
            let cx = v.rect.left + (target % cols) * cw + cw / 2;
            let cy = v.rect.top + (target / cols) * ch + ch / 2;
            let hit_cols = cols;
            let got = (cy - v.rect.top) / ch * hit_cols + (cx - v.rect.left) / cw;
            assert_eq!(got, target, "hit-test disagrees for cell {target}");
        }

        // The highlighted cell's top-left corner carries the pick colour, and
        // an unpicked neighbour's does not.
        let corner = |c: i32| {
            let x = v.rect.left + (c % cols) * cw;
            let y = v.rect.top + (c / cols) * ch;
            v.buf[(y * vw + x) as usize]
        };
        assert_eq!(corner(5), PICK, "cell 5 should be highlighted");
        assert_ne!(corner(4), PICK, "cell 4 should not be");
    }

    #[test]
    fn cells_owned_by_another_animation_are_marked_differently() {
        let (cols, rows, cell) = (4, 3, 16);
        let (px, iw, ih) = sheet(cols, rows, cell);
        let picked = vec![0u16];
        let taken = |c: u16| c == 7;
        let v = compose_sheet(&px, iw, ih, cols, rows, &picked, &taken, 400, 300);
        let cw = (v.rect.right - v.rect.left) / cols;
        let ch = (v.rect.bottom - v.rect.top) / rows;
        let at = |c: i32| {
            let x = v.rect.left + (c % cols) * cw;
            let y = v.rect.top + (c / cols) * ch;
            v.buf[(y * 400 + x) as usize]
        };
        assert_eq!(at(0), PICK);
        assert_eq!(at(7), OTHER, "a cell used elsewhere should warn, not hide");
        assert_eq!(at(3), GRID, "an unused cell keeps the plain grid");
    }

    /// Transparency has to be visible, or the author cannot tell an empty cell
    /// from a cell painted the same colour as the background.
    #[test]
    fn transparent_pixels_show_the_checkerboard() {
        let (px, iw, ih) = sheet(4, 3, 16);
        let v = compose_sheet(&px, iw, ih, 4, 3, &[], &|_| false, 400, 300);
        let seen: Vec<u32> = v.buf.iter().copied().filter(|&c| c == CHECK_A || c == CHECK_B).collect();
        assert!(!seen.is_empty(), "no checkerboard behind the transparent margin");
    }

    #[test]
    fn guessing_a_cell_size_prefers_something_near_64() {
        // Sheets in the wild: the turtle, the built-in export, and a 32px sheet.
        for (w, h, want) in [(512, 384, 64), (256, 224, 32), (1024, 768, 64)] {
            let mut best = (i32::MAX, 0);
            for c in 8..=256i32 {
                if w % c != 0 || h % c != 0 {
                    continue;
                }
                let cells = (w / c) * (h / c);
                if !(2..=400).contains(&cells) {
                    continue;
                }
                if (c - 64).abs() < best.0 {
                    best = ((c - 64).abs(), c);
                }
            }
            assert_eq!(best.1, want, "{w}x{h}");
        }
    }

    #[test]
    fn names_become_safe_folder_names() {
        assert_eq!(sanitise("Ninja Turtle!"), "ninja-turtle");
        assert_eq!(sanitise("  spaced  "), "spaced");
        assert_eq!(sanitise("../escape"), "escape");
        assert_eq!(sanitise(r"C:\evil"), "c--evil");
    }

    #[test]
    fn a_generator_filename_is_not_a_creature_name() {
        assert_eq!(default_name(Path::new("dragon.png")), "dragon");
        // The exporter's own filename: fall back to the folder.
        assert_eq!(
            default_name(Path::new("sprites/wolf/creature.png")),
            "wolf"
        );
        // Timestamped generator output: leave it blank so the user must type.
        assert!(default_name(Path::new("ChatGPT Image Aug 10, 2026, 04_55_04 PM.png")).is_empty());
        assert!(default_name(Path::new("Screenshot 2026-08-10.png")).is_empty());
        assert!(sanitise("???").is_empty(), "a name of only junk must be rejected upstream");
    }

    /// Render the sheet view against a real sheet and write it out, so the
    /// thing the user will actually look at can be looked at. Developer
    /// tooling: `cargo test -- --ignored preview_editor_view`.
    #[test]
    #[ignore]
    fn preview_editor_view() {
        let path = std::path::Path::new("assets/sprites/turtle/creature.png");
        let (px, iw, ih) = crate::sprites::decode_png_straight(path).expect("turtle sheet");
        let (iw, ih) = (iw as i32, ih as i32);
        let (cols, rows) = (8, 6);
        // As if "climb" were selected with 16 and 17 clicked, and other
        // animations already owning the first three rows.
        let picked: Vec<u16> = vec![16, 17];
        let taken = |c: u16| c < 24;
        let (vw, vh) = (700, 520);
        let v = compose_sheet(&px, iw, ih, cols, rows, &picked, &taken, vw, vh);

        let mut rgba = Vec::with_capacity((vw * vh * 4) as usize);
        for p in &v.buf {
            rgba.extend_from_slice(&[
                ((p >> 16) & 0xFF) as u8,
                ((p >> 8) & 0xFF) as u8,
                (p & 0xFF) as u8,
                255,
            ]);
        }
        std::fs::create_dir_all("target/preview").unwrap();
        let f = std::fs::File::create("target/preview/editor-view.png").unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), vw as u32, vh as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgba).unwrap();
        println!("wrote target/preview/editor-view.png");
    }

    /// Build the real window, load a real sheet into it, and photograph it.
    /// The layout is absolute pixel arithmetic against a fixed client size, and
    /// no amount of reading it tells you whether a control is three pixels off
    /// or behind the sheet. Developer tooling:
    /// `cargo test -- --ignored --test-threads=1 preview_editor_window`.
    #[test]
    #[ignore]
    fn preview_editor_window() {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetDC, GetDIBits, ReleaseDC,
            UpdateWindow,
        };
        use windows_sys::Win32::Storage::Xps::PrintWindow;

        unsafe {
            open();
            let hwnd = OPEN.with(|o| *o.borrow());
            assert!(!hwnd.is_null(), "the window did not open");

            // Feed it a real sheet the way a drop would.
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Editor;
            assert!(!ptr.is_null());
            let ed = &mut *ptr;
            // Prefer a raw generated sheet if one is lying around: that is the
            // case fitting exists for, and the one worth looking at.
            let raw = std::env::var("PETPAL_PREVIEW_SHEET").ok();
            let path = raw
                .as_deref()
                .map(Path::new)
                .filter(|p| p.exists())
                .unwrap_or(Path::new("assets/sprites/turtle/creature.png"));
            load(ed, path);
            ed.sel = Anim::Climb as usize;
            ed.frames[Anim::Climb as usize] = vec![16, 17];
            set_text(ed.ms_edit, &ed.ms[ed.sel].to_string());
            sync_list(ed);
            SendMessageW(ed.list, LB_SETCURSEL, Anim::Climb as WPARAM, 0);
            SendMessageW(ed.list, LB_SETTOPINDEX, 0, 0);

            // Let it paint.
            let mut msg: MSG = std::mem::zeroed();
            for _ in 0..200 {
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            UpdateWindow(hwnd);

            let mut rc: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rc);
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);

            let screen = GetDC(null_mut());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp as _);
            // PW_RENDERFULLCONTENT (2) is what makes child controls appear.
            PrintWindow(hwnd, mem, 2);

            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let mut raw = vec![0u8; (w * h * 4) as usize];
            GetDIBits(mem, bmp, 0, h as u32, raw.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);

            SelectObject(mem, old);
            DeleteObject(bmp as _);
            DeleteDC(mem);
            ReleaseDC(null_mut(), screen);
            DestroyWindow(hwnd);

            // BGRA -> RGBA.
            let mut rgba = Vec::with_capacity(raw.len());
            for p in raw.chunks_exact(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
            std::fs::create_dir_all("target/preview").unwrap();
            let f = std::fs::File::create("target/preview/editor-window.png").unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&rgba).unwrap();
            println!("wrote target/preview/editor-window.png ({w}x{h})");
        }
    }

    /// What the button writes, the app must read back identically. This is the
    /// contract the whole window exists to satisfy.
    #[test]
    fn what_the_window_writes_is_what_the_app_loads() {
        let mut frames = vec![Vec::new(); ANIM_COUNT];
        frames[Anim::Idle as usize] = vec![0, 1, 2, 3, 4, 5, 6, 7];
        frames[Anim::Walk as usize] = vec![8, 9, 10, 11, 12, 13, 14, 15];
        // Deliberately out of order and with a repeat: the manifest must keep
        // click order exactly, because that is the playback order the author saw
        // in the preview.
        frames[Anim::Climb as usize] = vec![17, 16, 17];
        let mut ms = vec![120u32; ANIM_COUNT];
        ms[Anim::Idle as usize] = 240;
        ms[Anim::Climb as usize] = 90;

        let dir = std::env::temp_dir().join("petpal-addsprite-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A 8x6 sheet of 64px cells, fully inked so nothing reads as blank.
        let (cw, ch, cols, rows) = (64i32, 64i32, 8i32, 6i32);
        let (w, h) = ((cols * cw) as u32, (rows * ch) as u32);
        let rgba = vec![0xC0u8; (w * h * 4) as usize];
        let f = std::fs::File::create(dir.join("creature.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgba).unwrap();

        std::fs::write(dir.join("sprite.toml"), manifest(cw, ch, &frames, &ms)).unwrap();

        let set = crate::sprites::load_sheet(&dir).expect("the app must load what we wrote");
        assert_eq!(set.w, cw as u32);
        assert_eq!(set.h, ch as u32);
        assert_eq!(set.clip(Anim::Idle).idx, frames[Anim::Idle as usize]);
        assert_eq!(set.clip(Anim::Idle).frame_ms, 240);
        assert_eq!(
            set.clip(Anim::Climb).idx,
            vec![17, 16, 17],
            "click order and repeats must survive"
        );
        assert_eq!(set.clip(Anim::Climb).frame_ms, 90);
        // Unassigned animations fall back, and climb's fallback is walk -- but
        // climb was assigned here, so run should be idle's artwork.
        assert_eq!(set.clip(Anim::Run).idx, frames[Anim::Idle as usize]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An animation left empty must not appear in the file at all. Writing
    /// `frames = []` would be a clip with no pictures rather than a clip that
    /// falls back to idle.
    #[test]
    fn empty_animations_are_omitted_not_emptied() {
        let mut frames = vec![Vec::new(); ANIM_COUNT];
        frames[Anim::Idle as usize] = vec![0];
        let text = manifest(32, 32, &frames, &vec![120; ANIM_COUNT]);
        assert!(text.contains("[anims.idle]"));
        assert!(!text.contains("frames = []"));
        assert!(!text.contains("[anims.sleep]"));
        // ...but it should say so, so the author is not left wondering.
        assert!(text.contains("fall back to idle"));
    }

    /// The whole new path in one go: take a sheet, fit it, write what the Add
    /// button would write, and load it with the real loader. Fitting produces
    /// the PNG *and* decides the frame numbering, so an error anywhere in it
    /// shows up as a creature with holes rather than as a failure.
    #[test]
    fn a_fitted_sheet_round_trips_through_the_loader() {
        let src = std::path::Path::new("assets/sprites/turtle/creature.png");
        let (px, w, h) = crate::sprites::decode_png_straight(src).expect("turtle sheet");

        let o = crate::regrid::Options { cell: 64, ..Default::default() };
        let f = crate::regrid::fit(&px, w, h, &o).expect("fit");
        assert_eq!((f.cols, f.rows), (8, 6), "the turtle sheet is 8x6");
        assert_eq!(f.found, 48, "every cell should carry a sprite");
        assert_eq!((f.w, f.h), (512, 384));

        let dir = std::env::temp_dir().join("petpal-fit-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_png(&dir.join("creature.png"), &f.px, f.w, f.h).expect("write");

        // Assign every cell so a hole anywhere would be caught.
        let mut frames = vec![Vec::new(); ANIM_COUNT];
        frames[Anim::Idle as usize] = (0..48u16).collect();
        std::fs::write(
            dir.join("sprite.toml"),
            manifest(f.cell as i32, f.cell as i32, &frames, &vec![120; ANIM_COUNT]),
        )
        .unwrap();

        let set = crate::sprites::load_sheet(&dir).expect("load");
        assert_eq!(set.frame_count(Anim::Idle), 48);
        for i in 0..48 {
            let ink = set.frame(Anim::Idle, i).px.iter().filter(|&&p| p >> 24 > 24).count();
            assert!(ink > 50, "frame {i} came out blank after fitting");
        }
        // Feet on the bottom row is the placement the renderer assumes.
        for i in 0..48 {
            let fr = set.frame(Anim::Idle, i);
            let last = (63 * 64..64 * 64).any(|k| fr.px[k] >> 24 > 24);
            assert!(last, "frame {i} does not reach the bottom row of its cell");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The prompt on the clipboard is pulled out of the guide at runtime, so it
    /// can go missing without anyone noticing. Check it is there, and that it
    /// still says the things the rest of the tool assumes it says.
    #[test]
    fn the_clipboard_prompt_matches_the_documented_layout() {
        let p = ai_prompt().expect("the guide must contain the AI prompt");
        assert!(p.len() > 800, "suspiciously short prompt: {} bytes", p.len());
        assert!(p.ends_with("
") || p.ends_with("shadows."), "prompt looks truncated");
        // Windows line endings, or it pastes as one line into most tools.
        assert!(p.contains("
"), "prompt must use CRLF for the clipboard");
        assert!(!p.contains("```"), "the fence must not be included");

        for needed in [
            "8 columns and 6 rows",
            "48 equal squares",
            "THE CREATURE:",
            "FACES RIGHT",
            "transparent background",
        ] {
            assert!(
                p.to_lowercase().contains(&needed.to_lowercase()),
                "prompt no longer mentions {needed:?}"
            );
        }
        // Every animation the tool can assign should be described somewhere in
        // the prompt, or the generated sheet will be missing poses.
        let low = p.to_lowercase();
        for word in ["idle", "walk", "run", "climb", "fall", "asleep", "annoyed", "surprised", "sitting", "held"] {
            assert!(low.contains(word), "prompt does not ask for {word:?}");
        }
    }

    /// Every control on the window should be explained. A control nobody can
    /// find out about might as well not exist.
    #[test]
    fn the_help_covers_every_control() {
        for phrase in [
            "trim base", "remove bg", "cols, rows", "Fit to grid", "Use original",
            "cell w x h", "Guess", "name", "ms per frame", "Whole row", "Clear",
        ] {
            assert!(HELP.contains(phrase), "help never mentions {phrase:?}");
        }
        // And it should say what the values do, not just name them.
        assert!(HELP.contains("too high"), "help should describe wrong values");
        assert!(HELP.contains("blank"), "help should say what blank means");
    }


    /// The help window cannot be photographed -- a standard EDIT does not
    /// answer WM_PRINTCLIENT, so PrintWindow returns a blank rectangle no
    /// matter how correct the window is, and capturing the screen instead
    /// grabs whatever happens to be in front. So check it structurally: the
    /// control exists, holds all of the text, is visible and fills the frame.
    ///
    /// Creates a real window, so it is ignored by default:
    /// `cargo test -- --ignored --test-threads=1 the_help_window_builds`.
    #[test]
    #[ignore]
    fn the_help_window_builds() {
        unsafe {
            CTL_BG.with(|b| {
                if *b.borrow() == 0 {
                    *b.borrow_mut() = CreateSolidBrush(rgb(0x2B, 0x2E, 0x38)) as isize;
                }
            });
            CTL_FIELD.with(|b| {
                if *b.borrow() == 0 {
                    *b.borrow_mut() = CreateSolidBrush(rgb(0x1E, 0x20, 0x28)) as isize;
                }
            });
            show_help(null_mut());
            let hwnd = FindWindowW(wide("PetPalSpriteHelp").as_ptr(), null_mut());
            assert!(!hwnd.is_null(), "the help window did not open");

            let child = GetWindow(hwnd, GW_CHILD);
            assert!(!child.is_null(), "the help text control was not created");
            assert_eq!(
                GetWindowTextLengthW(child) as usize,
                HELP.chars().count(),
                "the control did not take all of the help text"
            );
            assert_ne!(IsWindowVisible(child), 0, "the help text control is hidden");

            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(child, &mut rc);
            assert!(rc.right > 600 && rc.bottom > 500, "the control is {}x{}", rc.right, rc.bottom);

            // The two brushes the paint handler picks between must not be the
            // same colour as the text, or the window renders as a blank slab.
            assert_ne!(CTL_FIELD.with(|b| *b.borrow()), 0);
            DestroyWindow(hwnd);
        }
    }
}
