//! PetPal — a small animated creature that lives on your desktop.
//!
//! Design notes on cost, since a desktop pet has to be invisible in Task Manager:
//!
//! * **No render thread and no timer spam.** The main loop blocks in
//!   `MsgWaitForMultipleObjectsEx` until either input arrives or the creature's
//!   *own* next animation frame is due. A sleeping pet wakes ~3x a second and
//!   does nothing.
//! * **Frames are only pushed when something changed.** Position, facing,
//!   animation and frame index form a key; an unchanged key skips the blit and
//!   the kernel call entirely.
//! * **One allocation-free DIB.** A 32x32 sprite at 3x is a 36 KB buffer reused
//!   forever; drawing a frame is a fixed-point nearest-neighbour upscale.
//! * **Observation is polled, not hooked.** CPU load samples every 1.5 s, window
//!   geometry every 400 ms and only while awake, idle time once per wake-up.

#![windows_subsystem = "windows"]

mod behavior;
mod config;
mod monkey;
mod mouse;
mod platforms;
mod render;
mod reminders;
mod sheet;
mod sprites;
mod sysinfo;
mod tray;
mod vader;
mod win;

use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use behavior::{Ctx, Pet, State};
use config::Config;
use platforms::World;
use render::Canvas;
use sprites::{Anim, SpriteSet};
use sysinfo::CpuMonitor;
use win::{now_ms, wide, Rng};

/// Posted by the WinEvent hook when a window appears; `lparam` is the HWND.
const WM_APP_NEWWIN: u32 = WM_APP + 2;

/// How often to re-scan window geometry while the creature is awake.
const WORLD_SCAN_MS: u64 = 400;
/// CPU sampling period. Long enough to be free, short enough to feel reactive.
const CPU_SAMPLE_MS: u64 = 1_500;
/// Upper bound on how long the loop may sleep. Sets the latency for noticing
/// user input (waking up) and CPU spikes.
const MAX_IDLE_WAIT_MS: u32 = 300;
/// Minimum gap between "an app opened" reactions, so a burst of windows at
/// login does not make the creature flail.
const NOTICE_COOLDOWN_MS: u64 = 8_000;
/// A press shorter and stiller than this is a poke, not a drag.
const CLICK_SLOP_PX: i32 = 4;
const CLICK_MAX_MS: u64 = 400;
/// Alpha below which a pixel counts as click-through.
const HIT_ALPHA: u8 = 40;

/// Set once the window exists, so the WinEvent callback can post to it.
static APP_HWND: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());

struct Drag {
    /// Offset from the cursor to the pet's feet position at grab time.
    dx: i32,
    dy: i32,
    start_ms: u64,
    travelled: i32,
}

struct App {
    hwnd: HWND,
    cfg: Config,
    set: SpriteSet,
    canvas: Canvas,
    pet: Pet,
    world: World,
    cpu: CpuMonitor,
    sched: reminders::Scheduler,
    tray: tray::Tray,
    icon: HICON,
    rng: Rng,

    last_tick: u64,
    /// Everything that affects the presented image; unchanged means no redraw.
    last_key: Option<(i32, i32, u32, bool, u32)>,
    drag: Option<Drag>,
    last_notice: u64,
    /// Where the sprites came from, for the About box.
    sprite_source: String,
}

impl App {
    fn new(hwnd: HWND) -> App {
        let (cfg, cfg_err) = Config::load();
        let (set, sprite_source, sprite_err) = load_sprites(&cfg);
        let canvas = make_canvas(&set, cfg.scale);

        let mut world = World::new(hwnd);
        world.refresh(now_ms(), 0, true);

        // Start near the top-right of the primary monitor and drop in.
        let m = world.monitors[0];
        let mut pet = Pet::new(
            (m.work.right - 160).max(m.work.left + 40) as f32,
            (m.work.top + 80) as f32,
        );
        let kind = sprites::Kind::from_id(&cfg.sprite);
        pet.set_body_width(canvas.w, kind.map_or(0.375, |k| k.body_half_frac()));
        pet.set_speed_scale(kind.map_or(1.0, |k| k.speed_scale()));

        let icon = make_icon(&set);
        let tray = tray::Tray::new(hwnd, icon);
        if let Some(err) = cfg_err.or(sprite_err) {
            tray.notify("PetPal", &err);
        }

        App {
            hwnd,
            cfg,
            set,
            canvas,
            pet,
            world,
            cpu: CpuMonitor::new(CPU_SAMPLE_MS),
            sched: reminders::Scheduler::new(),
            tray,
            icon,
            rng: Rng::new(),
            last_tick: now_ms(),
            last_key: None,
            drag: None,
            last_notice: 0,
            sprite_source,
        }
    }

    /// Advance the simulation. Returns how long the loop may sleep, in ms.
    fn tick(&mut self) -> u32 {
        let now = now_ms();
        // Clamp dt so returning from machine sleep doesn't teleport the pet.
        let dt = (now - self.last_tick).min(250) as u32;
        if dt == 0 {
            return 1;
        }
        self.last_tick = now;

        self.cpu.poll(now);
        let idle = sysinfo::idle_ms();

        // Skip the window scan entirely while asleep; nothing will move the pet.
        if self.cfg.walk_on_windows && self.pet.state != State::Sleep {
            self.world.refresh(now, WORLD_SCAN_MS, false);
        }

        if let Some(text) = self.sched.poll(now, &self.cfg) {
            self.tray.notify("PetPal reminder", &text);
            self.pet.notice(None);
        }

        let cursor = cursor_pos();
        {
            let mut ctx = Ctx {
                world: &self.world,
                cursor: (cursor.x, cursor.y),
                cpu_load: self.cpu.load,
                idle_ms: idle,
                cfg: &self.cfg,
                rng: &mut self.rng,
            };
            self.pet.update(dt, &mut ctx, &self.set);
        }

        self.present();

        self.pet.next_wake_ms(&self.set).min(MAX_IDLE_WAIT_MS)
    }

    /// Blit and push the current frame, skipping the work if nothing moved.
    fn present(&mut self) {
        let anim = self.pet.anim();
        let mirror = self.pet.facing < 0;
        let x = self.pet.x.round() as i32 - self.canvas.w / 2;
        // The sprite's bottom row aligns with the surface the pet stands on.
        let y = self.pet.y.round() as i32 - self.canvas.h;

        let key = (x, y, self.pet.frame as u32, mirror, anim as u32);
        if self.last_key == Some(key) {
            return;
        }
        self.last_key = Some(key);

        let frame = self.set.frame(anim, self.pet.frame);
        self.canvas.draw_frame(frame, self.set.w, self.set.h, mirror);
        self.canvas.present(self.hwnd, x, y, self.cfg.opacity);
    }

    fn window_rect(&self) -> RECT {
        let x = self.pet.x.round() as i32 - self.canvas.w / 2;
        let y = self.pet.y.round() as i32 - self.canvas.h;
        RECT {
            left: x,
            top: y,
            right: x + self.canvas.w,
            bottom: y + self.canvas.h,
        }
    }

    // -- input -------------------------------------------------------------

    fn hit_test(&self, screen_x: i32, screen_y: i32) -> LRESULT {
        let rc = self.window_rect();
        let a = self.canvas.alpha_at(screen_x - rc.left, screen_y - rc.top);
        // Clicks land on the creature itself; the rest of the square window
        // passes them through to whatever is underneath.
        if a >= HIT_ALPHA {
            HTCLIENT as LRESULT
        } else {
            HTTRANSPARENT as LRESULT
        }
    }

    fn begin_drag(&mut self) {
        let p = cursor_pos();
        self.drag = Some(Drag {
            dx: self.pet.x.round() as i32 - p.x,
            dy: self.pet.y.round() as i32 - p.y,
            start_ms: now_ms(),
            travelled: 0,
        });
        self.pet.begin_drag();
        unsafe { SetCapture(self.hwnd) };
    }

    fn drag_move(&mut self) {
        let Some(d) = self.drag.as_mut() else { return };
        let p = cursor_pos();
        let nx = (p.x + d.dx) as f32;
        let ny = (p.y + d.dy) as f32;
        d.travelled += ((nx - self.pet.x).abs() + (ny - self.pet.y).abs()) as i32;
        self.pet.drag_to(nx, ny);
        self.present();
    }

    fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        unsafe { ReleaseCapture() };
        self.pet.end_drag();
        let held = now_ms().saturating_sub(d.start_ms);
        if d.travelled <= CLICK_SLOP_PX && held <= CLICK_MAX_MS {
            // A poke rather than a drag: react in place instead of falling.
            self.pet.notice(None);
        }
    }

    /// A top-level window just appeared.
    fn on_new_window(&mut self, hwnd: HWND) {
        if !self.cfg.react_to_new_apps || hwnd.is_null() {
            return;
        }
        let now = now_ms();
        if now.saturating_sub(self.last_notice) < NOTICE_COOLDOWN_MS {
            return;
        }
        if self.pet.is_dragging() || self.pet.state == State::Annoyed {
            return;
        }
        // Only real, standable application windows count.
        let Some(rc) = platforms::standable_rect(hwnd) else {
            return;
        };
        if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
            return;
        }

        self.last_notice = now;
        self.pet.notice(Some((rc.left + rc.right) / 2));

        let title = platforms::window_title(hwnd);
        if !title.is_empty() {
            self.tray.notify("PetPal", &format!("Ooh — {title}"));
        }
        // The new window changes the scenery, so refresh ledges immediately.
        self.world.refresh(now, 0, true);
    }

    // -- menu --------------------------------------------------------------

    /// Every sprite the user can pick: built-in creatures first, then any sheet
    /// folders installed under `<config>/sprites/`.
    fn sprite_choices(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = sprites::Kind::ALL
            .iter()
            .map(|k| (k.label().to_string(), k.id().to_string()))
            .collect();
        for name in Config::installed_sheets() {
            out.push((name.clone(), name));
        }
        // A sheet configured by absolute path still needs to show as selected.
        let current = self.cfg.sprite.trim();
        if !current.is_empty() && !out.iter().any(|(_, id)| id == current) {
            out.push((format!("{current} (from config)"), current.to_string()));
        }
        out
    }

    /// Show the tray/context menu and keep it open across changes.
    ///
    /// A Win32 popup menu closes on every click, which makes toggling three
    /// settings a three-right-click chore. Reopening it at the same anchor
    /// turns it into a small settings panel: checkmarks update in place and the
    /// user leaves via "Close menu", Escape, or a click elsewhere.
    fn show_menu(&mut self) {
        let anchor = tray::cursor_anchor();
        loop {
            let st = tray::MenuState {
                chase: self.cfg.chase_cursor,
                walk_on_windows: self.cfg.walk_on_windows,
                jump_between_windows: self.cfg.jump_between_windows,
                react: self.cfg.react_to_new_apps,
                asleep: self.pet.is_sleeping(),
                scale: self.cfg.scale,
                roam: self.cfg.roam,
                sleep_after_idle_secs: self.cfg.sleep_after_idle_secs,
                sprites: self.sprite_choices(),
                sprite: self.cfg.sprite.clone(),
            };
            let cmd = tray::show_menu(self.hwnd, anchor, &st);
            if cmd == 0 || cmd == tray::CMD_CLOSE_MENU {
                return;
            }
            self.on_command(cmd);
            if !tray::keeps_menu_open(cmd) {
                return;
            }
        }
    }

    fn on_command(&mut self, cmd: u32) {
        use tray::*;
        let mut save = true;
        match cmd {
            CMD_CHASE => self.cfg.chase_cursor = !self.cfg.chase_cursor,
            CMD_WINDOWS => {
                self.cfg.walk_on_windows = !self.cfg.walk_on_windows;
                if self.cfg.walk_on_windows {
                    self.world.refresh(now_ms(), 0, true);
                } else {
                    self.world.floors_only();
                }
            }
            CMD_JUMP => {
                self.cfg.jump_between_windows = !self.cfg.jump_between_windows
            }
            CMD_REACT => self.cfg.react_to_new_apps = !self.cfg.react_to_new_apps,
            CMD_SLEEP => {
                self.pet.force_sleep();
                save = false;
            }
            CMD_WAKE => {
                self.pet.wake();
                save = false;
            }
            CMD_COME => {
                let p = cursor_pos();
                self.pet.teleport(p.x as f32, p.y as f32);
                save = false;
            }
            CMD_OPEN_DIR => {
                let _ = std::fs::create_dir_all(Config::dir());
                open_folder(&Config::dir());
                save = false;
            }
            CMD_EXPORT_SHEET => {
                self.export_current_sheet();
                save = false;
            }
            CMD_OPEN_SPRITES => {
                let _ = std::fs::create_dir_all(Config::sprites_dir());
                open_folder(&Config::sprites_dir());
                save = false;
            }
            CMD_RELOAD => {
                self.reload();
                save = false;
            }
            CMD_ABOUT => {
                self.show_about();
                save = false;
            }
            CMD_EXIT => {
                unsafe { PostQuitMessage(0) };
                save = false;
            }
            n if (CMD_SCALE_BASE + 1..=CMD_SCALE_BASE + 6).contains(&n) => {
                self.cfg.scale = n - CMD_SCALE_BASE;
                self.rebuild_canvas();
            }
            n if (CMD_SPRITE_BASE..CMD_ROAM_BASE).contains(&n) => {
                let i = (n - CMD_SPRITE_BASE) as usize;
                match self.sprite_choices().get(i) {
                    Some((_, id)) => {
                        self.cfg.sprite = id.clone();
                        if let Some(e) = self.apply_sprites() {
                            self.tray.notify("PetPal", &e);
                        }
                    }
                    None => save = false,
                }
            }
            n if (CMD_ROAM_BASE..CMD_SLEEP_BASE).contains(&n) => {
                let i = (n - CMD_ROAM_BASE) as usize;
                match tray::ROAM_PRESETS.get(i) {
                    Some((_, value)) => self.cfg.roam = *value,
                    None => save = false,
                }
            }
            n if n >= CMD_SLEEP_BASE => {
                let i = (n - CMD_SLEEP_BASE) as usize;
                match tray::SLEEP_PRESETS.get(i) {
                    Some((_, secs)) => {
                        self.cfg.sleep_after_idle_secs = *secs;
                        // Choosing a timer implies wanting the timer, not the
                        // nap you asked for a moment ago.
                        self.pet.wake();
                    }
                    None => save = false,
                }
            }
            _ => save = false,
        }
        if save {
            // A toggle that cannot be persisted still applies for this session,
            // but the user needs to know it will not survive a restart.
            if let Err(e) = self.cfg.save() {
                let msg = format!("Setting applied, but not saved: {e}");
                self.tray.notify("PetPal", &msg);
            }
        }
    }

    /// Rebuild the sprite set from the current config and refresh everything
    /// that depends on it: the canvas size, the pet's body width, and the tray
    /// icon. Returns a load error if the configured sheet could not be used.
    fn apply_sprites(&mut self) -> Option<String> {
        let (set, source, err) = load_sprites(&self.cfg);
        self.set = set;
        self.sprite_source = source;
        // Built-ins each have their own pace; a user sheet just gets the
        // configured speed.
        self.pet.set_speed_scale(
            sprites::Kind::from_id(&self.cfg.sprite).map_or(1.0, |k| k.speed_scale()),
        );
        self.rebuild_canvas();

        render::destroy_icon(self.icon);
        self.icon = make_icon(&self.set);
        self.tray.set_icon(self.icon);
        err
    }

    /// Write the current creature out as an editable sprite sheet.
    ///
    /// This is the on-ramp for a custom creature: the exported folder is a
    /// working sprite the app can already load, so the user edits pixels rather
    /// than starting from an empty grid and guessing the layout.
    fn export_current_sheet(&mut self) {
        let base = Config::sprites_dir().join(unique_export_name(&self.cfg.sprite));
        match sheet::export_sheet(&self.set, &base) {
            Ok(()) => {
                let name = base
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let msg = format!(
                    "Copied to sprites/{name}. Edit creature.png, then pick \
                     \"{name}\" from Tray > Sprite."
                );
                self.tray.notify("PetPal", &msg);
                open_folder(&base);
            }
            Err(e) => self.tray.notify("PetPal", &format!("Export failed: {e}")),
        }
    }

    fn reload(&mut self) {
        let (cfg, cfg_err) = Config::load();
        self.cfg = cfg;
        let sprite_err = self.apply_sprites();
        self.world.refresh(now_ms(), 0, true);

        let msg = match (cfg_err, sprite_err) {
            (Some(e), _) => e,
            (None, Some(e)) => e,
            (None, None) => format!("Reloaded ({})", self.sprite_source),
        };
        self.tray.notify("PetPal", &msg);
    }

    fn rebuild_canvas(&mut self) {
        self.canvas = make_canvas(&self.set, self.cfg.scale);
        let frac = sprites::Kind::from_id(&self.cfg.sprite).map_or(0.375, |k| k.body_half_frac());
        self.pet.set_body_width(self.canvas.w, frac);
        self.last_key = None; // force a repaint at the new size
        self.present();
    }

    fn show_about(&self) {
        let text = format!(
            "PetPal {}\n\n\
             Sprites: {}\n\
             Frames: {} ({} KB of pixels)\n\
             Window: {}x{} px\n\
             Working set: {} KB\n\
             CPU load seen: {:.0}%\n\
             Ledges tracked: {}\n\n\
             Config: {}\n\n\
             Drag the creature to pick it up and throw it.\n\
             Click it for a reaction. Right-click for this menu.",
            env!("CARGO_PKG_VERSION"),
            self.sprite_source,
            self.set.frames.len(),
            self.set.bytes() / 1024,
            self.canvas.w,
            self.canvas.h,
            sysinfo::working_set_bytes() / 1024,
            self.cpu.load * 100.0,
            self.world.ledges.len(),
            Config::path().display(),
        );
        let title = wide("About PetPal");
        let body = wide(&text);
        unsafe {
            MessageBoxW(
                null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            )
        };
    }
}

impl Drop for App {
    fn drop(&mut self) {
        render::destroy_icon(self.icon);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cursor_pos() -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    p
}

fn make_canvas(set: &SpriteSet, scale: u32) -> Canvas {
    let s = scale.clamp(1, 8) as i32;
    Canvas::new(set.w as i32 * s, set.h as i32 * s)
}

fn make_icon(set: &SpriteSet) -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    render::icon_from_frame(set.frame(Anim::Idle, 0), set.w, set.h, size)
}

/// Load the user's sprite sheet if configured, else draw the built-in creature.
/// Returns the set and a short description for the About box.
fn load_sprites(cfg: &Config) -> (SpriteSet, String, Option<String>) {
    let fallback = |note: Option<String>| {
        let kind = sprites::Kind::Pal;
        let pal = cfg.colors.apply(kind.palette());
        (sprites::builtin(kind, &pal), kind.label().to_string(), note)
    };

    match cfg.sheet_path() {
        // A user sheet, by folder name or path.
        Some(dir) => match sprites::load_sheet(&dir) {
            Ok(set) => (set, dir.display().to_string(), None),
            // Fall back rather than refusing to run; the caller surfaces the
            // message through the tray.
            Err(e) => fallback(Some(format!("Could not load that sprite sheet: {e}"))),
        },
        // A built-in creature.
        None => match sprites::Kind::from_id(&cfg.sprite) {
            Some(kind) => {
                let pal = cfg.colors.apply(kind.palette());
                (sprites::builtin(kind, &pal), kind.label().to_string(), None)
            }
            None => fallback(Some(format!("Unknown sprite \"{}\"", cfg.sprite))),
        },
    }
}

/// A folder name under `sprites/` that is not already taken.
fn unique_export_name(sprite: &str) -> String {
    let stem = sprite.trim();
    let stem = if stem.is_empty() { "pal" } else { stem };
    // A configured path would make a terrible folder name; use its last segment.
    let stem = std::path::Path::new(stem)
        .file_name()
        .map_or(stem, |n| n.to_str().unwrap_or(stem));

    let dir = Config::sprites_dir();
    let first = format!("{stem}-copy");
    if !dir.join(&first).exists() {
        return first;
    }
    for n in 2..1000 {
        let candidate = format!("{stem}-copy{n}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    first
}

fn open_folder(path: &std::path::Path) {
    let verb = wide("open");
    let p = wide(&path.to_string_lossy());
    unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            p.as_ptr(),
            null_mut(),
            null_mut(),
            SW_SHOWNORMAL as i32,
        )
    };
}

/// Run `f` against the `App` stashed in the window's user data.
///
/// The pointer is installed right after `CreateWindowEx` and cleared before the
/// `App` is dropped, so any call reached through the message loop sees a live
/// object; messages sent during window creation see nothing.
///
/// The `RefCell` is not paranoia. `TrackPopupMenu` and `MessageBoxW` run their
/// own modal message loops, so a menu or the About box will re-enter this
/// window procedure while an outer borrow is still live. Handing out a second
/// `&mut App` there would be undefined behaviour; instead the re-entrant
/// message is skipped and falls through to `DefWindowProc`.
fn with_app<R>(hwnd: HWND, f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<App>;
    if p.is_null() {
        return None;
    }
    // SAFETY: the pointer refers to a `RefCell<App>` owned by `main`, which
    // outlives the message loop that reaches this function.
    let cell = unsafe { &*p };
    let mut borrow = cell.try_borrow_mut().ok()?;
    Some(f(&mut borrow))
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Explorer restarting destroys every tray icon; this message tells us to
    // put ours back.
    if msg == taskbar_created_msg() {
        with_app(hwnd, |app| app.tray.add());
        return 0;
    }

    match msg {
        WM_NCHITTEST => {
            // lParam is in screen coordinates for this message.
            let x = (lparam & 0xffff) as i16 as i32;
            let y = ((lparam >> 16) & 0xffff) as i16 as i32;
            if let Some(hit) = with_app(hwnd, |app| app.hit_test(x, y)) {
                return hit;
            }
        }
        WM_LBUTTONDOWN => {
            with_app(hwnd, |app| app.begin_drag());
            return 0;
        }
        WM_MOUSEMOVE => {
            with_app(hwnd, |app| {
                if app.drag.is_some() {
                    app.drag_move();
                }
            });
            return 0;
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            with_app(hwnd, |app| app.end_drag());
            return 0;
        }
        WM_RBUTTONUP => {
            with_app(hwnd, |app| app.show_menu());
            return 0;
        }
        tray::WM_TRAY => {
            let event = (lparam & 0xffff) as u32;
            with_app(hwnd, |app| match event {
                WM_RBUTTONUP | WM_CONTEXTMENU => app.show_menu(),
                WM_LBUTTONDBLCLK => {
                    // Double-click the tray icon to fetch the pet.
                    let p = cursor_pos();
                    app.pet.teleport(p.x as f32, (p.y - 60) as f32);
                }
                WM_LBUTTONUP => app.pet.notice(None),
                _ => {}
            });
            return 0;
        }
        WM_APP_NEWWIN => {
            with_app(hwnd, |app| app.on_new_window(lparam as HWND));
            return 0;
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
            with_app(hwnd, |app| app.world.refresh(now_ms(), 0, true));
            return 0;
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn taskbar_created_msg() -> u32 {
    // Registered messages are per-session constants, so this is a cheap lookup
    // after the first call.
    static MSG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MSG.get_or_init(|| {
        let name = wide("TaskbarCreated");
        unsafe { RegisterWindowMessageW(name.as_ptr()) }
    })
}

unsafe extern "system" fn win_event_proc(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // OBJID_WINDOW with no child means "the window itself", filtering out the
    // flood of events for menus, tooltips and client-area objects.
    if id_object != 0 || id_child != 0 || hwnd.is_null() {
        return;
    }
    let app = APP_HWND.load(Ordering::Relaxed);
    if app.is_null() {
        return;
    }
    // Do the real (more expensive) filtering on the main thread.
    unsafe { PostMessageW(app, WM_APP_NEWWIN, 0, hwnd as LPARAM) };
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    unsafe {
        // Physical pixels everywhere, so window rects and cursor positions all
        // agree regardless of per-monitor scaling.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    if already_running() {
        return;
    }

    let hinstance = unsafe { GetModuleHandleW(null_mut()) };
    let class_name = wide("PetPalWindow");

    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance as _,
            // Resource id 1, embedded by build.rs. Cast, not a string: this is
            // the MAKEINTRESOURCE idiom.
            hIcon: LoadIconW(hinstance as _, 1 as *const u16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null_mut(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: LoadIconW(hinstance as _, 1 as *const u16),
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }

        let title = wide("PetPal");
        let hwnd = CreateWindowExW(
            // LAYERED gives per-pixel alpha; NOACTIVATE keeps the creature from
            // ever stealing focus; TOOLWINDOW keeps it out of Alt-Tab.
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            64,
            64,
            null_mut(),
            null_mut(),
            hinstance as _,
            null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        APP_HWND.store(hwnd, Ordering::Relaxed);

        let app = RefCell::new(App::new(hwnd));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &app as *const RefCell<App> as isize);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        app.borrow_mut().present();

        // EVENT_OBJECT_SHOW out-of-context: the callback runs on this thread
        // while it pumps messages, so there is no cross-thread state at all.
        let hook = SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_SHOW,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );

        run_loop(&app);

        // Tear down in order: stop callbacks, unhook the pointer, then let App
        // drop (removing the tray icon and freeing GDI objects).
        if !hook.is_null() {
            UnhookWinEvent(hook);
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        APP_HWND.store(null_mut(), Ordering::Relaxed);
        drop(app);
        DestroyWindow(hwnd);
    }
}

/// Block until the pet needs redrawing or something happens.
fn run_loop(app: &RefCell<App>) {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // A modal loop (menu, About box) can be pumping messages through
            // us with the borrow held; skip the tick rather than panic.
            let wait = match app.try_borrow_mut() {
                Ok(mut a) => a.tick().max(1),
                Err(_) => MAX_IDLE_WAIT_MS,
            };

            // MWMO_INPUTAVAILABLE covers input that arrived between the drain
            // above and this call, which would otherwise wait for the timeout.
            MsgWaitForMultipleObjectsEx(0, null_mut(), wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// Named mutex guard. The handle intentionally leaks: it lives as long as the
/// process and Windows releases it on exit.
fn already_running() -> bool {
    let name = wide("Local\\PetPal-SingleInstance");
    unsafe {
        let h = CreateMutexW(null_mut(), 1, name.as_ptr());
        h.is_null() || GetLastError() == ERROR_ALREADY_EXISTS
    }
}
