//! Sprite storage, the built-in procedurally drawn creature, and the loader for
//! user-supplied sprite sheets.
//!
//! Pixels are stored as **premultiplied** BGRA packed into a `u32`
//! (`0xAARRGGBB` in host order), which is exactly what `UpdateLayeredWindow`
//! wants, so presenting a frame is a straight memcpy-style blit with no
//! per-frame conversion.

use std::path::Path;

pub const ANIM_COUNT: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anim {
    Idle = 0,
    Walk = 1,
    Run = 2,
    Fall = 3,
    Sleep = 4,
    Annoyed = 5,
    Drag = 6,
    Alert = 7,
    Sit = 8,
    Climb = 9,
}

impl Anim {
    pub const ALL: [Anim; ANIM_COUNT] = [
        Anim::Idle,
        Anim::Walk,
        Anim::Run,
        Anim::Fall,
        Anim::Sleep,
        Anim::Annoyed,
        Anim::Drag,
        Anim::Alert,
        Anim::Sit,
        Anim::Climb,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Anim::Idle => "idle",
            Anim::Walk => "walk",
            Anim::Run => "run",
            Anim::Fall => "fall",
            Anim::Sleep => "sleep",
            Anim::Annoyed => "annoyed",
            Anim::Drag => "drag",
            Anim::Alert => "alert",
            Anim::Sit => "sit",
            Anim::Climb => "climb",
        }
    }
}

pub struct Frame {
    pub px: Vec<u32>,
}

/// One animation: indices into `SpriteSet::frames` plus a per-frame duration.
/// Indices (rather than owned frames) let several clips share artwork without
/// duplicating pixel buffers.
pub struct Clip {
    pub idx: Vec<u16>,
    pub frame_ms: u32,
}

pub struct SpriteSet {
    pub w: u32,
    pub h: u32,
    pub frames: Vec<Frame>,
    pub clips: Vec<Clip>,
}

impl SpriteSet {
    pub fn clip(&self, a: Anim) -> &Clip {
        &self.clips[a as usize]
    }

    pub fn frame(&self, a: Anim, i: usize) -> &Frame {
        let clip = self.clip(a);
        let idx = clip.idx[i % clip.idx.len()] as usize;
        &self.frames[idx]
    }

    pub fn frame_count(&self, a: Anim) -> usize {
        self.clip(a).idx.len()
    }

    /// Approximate resident size of the pixel data, for the About box.
    pub fn bytes(&self) -> usize {
        self.frames.len() * (self.w * self.h) as usize * 4
    }
}

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

/// Straight (non-premultiplied) ARGB.
#[inline]
pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Convert straight ARGB to the premultiplied form the layered window needs.
#[inline]
pub fn premultiply(c: u32) -> u32 {
    let a = (c >> 24) & 0xff;
    if a == 255 {
        return c;
    }
    if a == 0 {
        return 0;
    }
    let r = (((c >> 16) & 0xff) * a + 127) / 255;
    let g = (((c >> 8) & 0xff) * a + 127) / 255;
    let b = ((c & 0xff) * a + 127) / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Parse `#rrggbb` / `#aarrggbb` (with or without the `#`). Returns straight ARGB.
pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    match s.len() {
        6 => Some(0xff00_0000 | v),
        8 => Some(v),
        _ => None,
    }
}

/// Same colour at reduced opacity.
pub(crate) fn fade(c: u32, alpha: f32) -> u32 {
    let a = (((c >> 24) & 0xff) as f32 * alpha.clamp(0.0, 1.0)) as u32;
    (a << 24) | (c & 0x00ff_ffff)
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Palette {
    pub body: u32,
    pub belly: u32,
    pub outline: u32,
    pub eye: u32,
    pub blush: u32,
    pub accent: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            body: argb(255, 0xF2, 0xA6, 0x5A),
            belly: argb(255, 0xFF, 0xE3, 0xC6),
            outline: argb(255, 0x3A, 0x2A, 0x1E),
            eye: argb(255, 0x2A, 0x1F, 0x18),
            blush: argb(255, 0xF0, 0x8B, 0x7A),
            accent: argb(255, 0xFF, 0xFF, 0xFF),
        }
    }
}

// ---------------------------------------------------------------------------
// Rasteriser
// ---------------------------------------------------------------------------

pub(crate) struct Raster<'a> {
    px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
}

impl<'a> Raster<'a> {
    /// Source-over composite of a straight-ARGB colour into the premultiplied buffer.
    #[inline]
    pub(crate) fn put(&mut self, x: i32, y: i32, c: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let sa = (c >> 24) & 0xff;
        if sa == 0 {
            return;
        }
        let i = (y * self.w + x) as usize;
        let src = premultiply(c);
        if sa == 255 {
            self.px[i] = src;
            return;
        }
        let dst = self.px[i];
        let inv = 255 - sa;
        let ch = |sh: u32| {
            let s = (src >> sh) & 0xff;
            let d = (dst >> sh) & 0xff;
            (s + (d * inv + 127) / 255).min(255)
        };
        self.px[i] = (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0);
    }

    pub(crate) fn hline(&mut self, x0: i32, x1: i32, y: i32, c: u32) {
        for x in x0..=x1 {
            self.put(x, y, c);
        }
    }

    pub(crate) fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
        for y in y0..=y1 {
            self.hline(x0, x1, y, c);
        }
    }

    pub(crate) fn ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: u32) {
        if rx < 0 || ry < 0 {
            return;
        }
        let (frx, fry) = (rx as f32 + 0.5, ry as f32 + 0.5);
        for dy in -ry..=ry {
            let t = 1.0 - (dy as f32 / fry).powi(2);
            if t <= 0.0 {
                continue;
            }
            let hw = (frx * t.sqrt()) as i32;
            self.hline(cx - hw, cx + hw, cy + dy, c);
        }
    }

    /// Filled ellipse with a 1px darker border, the workhorse for the creature's
    /// chunky pixel-art look.
    pub(crate) fn blob(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, fill: u32, line: u32) {
        self.ellipse(cx, cy, rx + 1, ry + 1, line);
        self.ellipse(cx, cy, rx, ry, fill);
    }

    /// Filled rectangle with a 1px border — the rectilinear counterpart to
    /// `blob`, for armour plates and panels.
    pub(crate) fn boxed(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, fill: u32, line: u32) {
        self.rect(x0, y0, x1, y1, line);
        if x1 - x0 >= 2 && y1 - y0 >= 2 {
            self.rect(x0 + 1, y0 + 1, x1 - 1, y1 - 1, fill);
        }
    }

    /// A tapering horn shape from a base to a tip — used for ears.
    pub(crate) fn taper(&mut self, bx: i32, by: i32, tx: i32, ty: i32, base_w: i32, c: u32) {
        const STEPS: i32 = 8;
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let x = bx as f32 + (tx - bx) as f32 * t;
            let y = by as f32 + (ty - by) as f32 * t;
            let w = (base_w as f32 * (1.0 - t)).round() as i32;
            self.hline(x as i32 - w, x as i32 + w, y.round() as i32, c);
        }
    }
}

// ---------------------------------------------------------------------------
// Pose description
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Eyes {
    Open,
    Half,
    Closed,
    Angry,
    Wide,
    Happy,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Legs {
    Stand,
    Walk(u8),
    Tuck,
    Splay,
    Dangle,
    /// Braced against a wall in front, hauling upward in antiphase.
    Cling(u8),
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Fx {
    None,
    Zzz(u8),
    Steam(u8),
    Bang(u8),
}

#[derive(Clone, Copy)]
pub(crate) struct Pose {
    pub body_dy: i32,
    pub squash: i32,
    pub head_dx: i32,
    pub head_dy: i32,
    pub lean: i32,
    pub legs: Legs,
    pub tail: i32,
    pub eyes: Eyes,
    pub ears_back: bool,
    pub lying: bool,
    pub blush: bool,
    pub fx: Fx,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            body_dy: 0,
            squash: 0,
            head_dx: 0,
            head_dy: 0,
            lean: 0,
            legs: Legs::Stand,
            tail: 0,
            eyes: Eyes::Open,
            ears_back: false,
            lying: false,
            blush: false,
            fx: Fx::None,
        }
    }
}

// Canvas layout for the 32x32 built-in creature.
//
// `FEET` is the centre row of the paws; their outline reaches row 31, the last
// row of the frame. The renderer aligns a frame's bottom edge with the ledge,
// so the paws land exactly on the surface with no per-sprite fudge factor.
pub(crate) const CW: i32 = 32;
pub(crate) const CH: i32 = 32;
pub(crate) const FEET: i32 = 29;

fn draw_pose(r: &mut Raster, p: &Pose, pal: &Palette) {
    if p.lying {
        draw_lying(r, p, pal);
    } else {
        draw_standing(r, p, pal);
    }
    draw_fx(r, p, pal);
}

fn draw_standing(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = CW / 2 + p.lean;
    let by = 22 + p.body_dy;
    let bry = 6 + p.squash;

    draw_tail(r, ox - 7, by + 1, p.tail, pal);
    draw_legs(r, ox, p.legs, pal);

    // Body, with a lighter belly patch on the facing side.
    r.blob(ox, by, 8, bry, pal.body, pal.outline);
    r.ellipse(ox + 2, by + 2, 4, bry - 3, pal.belly);

    let hx = ox + p.head_dx;
    let hy = 13 + p.head_dy + p.body_dy / 2;
    draw_head(r, hx, hy, p, pal);
}

fn draw_lying(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = CW / 2 + 2;
    let by = FEET - 3 + p.body_dy;
    let bry = 4 + p.squash;

    // Tail curls back over the body when asleep.
    draw_tail(r, ox + 7, by - 1, -p.tail - 2, pal);

    r.blob(ox, by, 9, bry, pal.body, pal.outline);
    r.ellipse(ox, by + 1, 6, bry - 2, pal.belly);

    // Tucked paws poking out at the front.
    r.blob(ox - 6, by + bry - 1, 2, 1, pal.belly, pal.outline);
    r.blob(ox - 2, by + bry - 1, 2, 1, pal.belly, pal.outline);

    let hx = ox - 8 + p.head_dx;
    let hy = by - 4 + p.head_dy;
    draw_head(r, hx, hy, p, pal);
}

fn draw_head(r: &mut Raster, hx: i32, hy: i32, p: &Pose, pal: &Palette) {
    // Ears first so the head outline overlaps their bases cleanly.
    for &dir in &[-1i32, 1] {
        let bx = hx + dir * 4;
        let by = hy - 4;
        let (tx, ty) = if p.ears_back {
            (bx + dir * 5, by + 1)
        } else {
            (bx + dir * 2, by - 6)
        };
        r.taper(bx, by, tx, ty, 3, pal.outline);
        r.taper(bx, by, tx, ty - 1, 2, pal.body);
    }

    r.blob(hx, hy, 6, 6, pal.body, pal.outline);

    // Eyes sit slightly forward of centre to suggest a facing direction.
    let ex = hx + 1;
    draw_eyes(r, ex, hy - 1, p.eyes, pal);

    if p.blush {
        r.ellipse(ex - 5, hy + 2, 1, 0, fade(pal.blush, 0.75));
        r.ellipse(ex + 5, hy + 2, 1, 0, fade(pal.blush, 0.75));
    }

    draw_mouth(r, ex, hy + 3, p.eyes, pal);
}

fn draw_eyes(r: &mut Raster, cx: i32, cy: i32, e: Eyes, pal: &Palette) {
    let hi = pal.accent;
    for &dx in &[-3i32, 3] {
        let x = cx + dx;
        match e {
            Eyes::Open => {
                r.ellipse(x, cy, 1, 1, pal.eye);
                r.put(x, cy - 1, hi);
            }
            Eyes::Wide => {
                r.ellipse(x, cy, 2, 2, pal.accent);
                r.ellipse(x, cy, 1, 1, pal.eye);
                r.put(x - 1, cy - 1, hi);
            }
            Eyes::Half => {
                r.hline(x - 1, x + 1, cy, pal.eye);
                r.put(x, cy + 1, pal.eye);
            }
            Eyes::Closed => {
                r.hline(x - 1, x + 1, cy, pal.eye);
                r.put(x - 2, cy - 1, pal.eye);
                r.put(x + 2, cy - 1, pal.eye);
            }
            Eyes::Angry => {
                // Brow slopes down toward the nose. Kept to single pixels and
                // held one row clear of the eye, or at 32px the whole face
                // merges into one dark block.
                let s = if dx < 0 { 1 } else { -1 };
                r.put(x - s * 2, cy - 3, pal.eye);
                r.put(x - s, cy - 2, pal.eye);
                r.put(x, cy - 2, pal.eye);
                r.put(x + s, cy - 1, pal.eye);
                r.put(x, cy + 1, pal.eye);
                r.put(x - s, cy + 1, pal.eye);
            }
            Eyes::Happy => {
                // '^' shaped closed-happy eyes.
                r.put(x - 2, cy + 1, pal.eye);
                r.put(x - 1, cy, pal.eye);
                r.put(x, cy - 1, pal.eye);
                r.put(x + 1, cy, pal.eye);
                r.put(x + 2, cy + 1, pal.eye);
            }
        }
    }
}

fn draw_mouth(r: &mut Raster, cx: i32, cy: i32, e: Eyes, pal: &Palette) {
    match e {
        Eyes::Angry => {
            // A frown: the mirror of the happy mouth below.
            r.put(cx - 2, cy + 1, pal.eye);
            r.hline(cx - 1, cx + 1, cy, pal.eye);
            r.put(cx + 2, cy + 1, pal.eye);
        }
        Eyes::Closed => {
            // Small open snoring mouth.
            r.hline(cx - 1, cx, cy, pal.eye);
        }
        Eyes::Happy | Eyes::Wide => {
            r.put(cx - 2, cy, pal.eye);
            r.hline(cx - 1, cx + 1, cy + 1, pal.eye);
            r.put(cx + 2, cy, pal.eye);
        }
        _ => {
            r.put(cx - 1, cy, pal.eye);
            r.put(cx + 1, cy, pal.eye);
            r.put(cx, cy + 1, pal.eye);
        }
    }
}

fn draw_tail(r: &mut Raster, bx: i32, by: i32, sway: i32, pal: &Palette) {
    let pts = [
        (bx, by),
        (bx - 3, by - 1 + sway),
        (bx - 5, by - 4 + sway * 2),
        (bx - 5, by - 7 + sway),
    ];
    for &(x, y) in &pts {
        r.blob(x, y, 1, 1, pal.body, pal.outline);
    }
}

/// One stride of the shared eight-phase gait, as a horizontal offset.
pub(crate) const SWING: [i32; 8] = [0, 2, 3, 2, 0, -2, -3, -2];

/// How far a paw lifts at a given point in its swing: highest mid-stride,
/// planted at the extremes.
pub(crate) fn lift_of(swing: i32) -> i32 {
    match swing.abs() {
        3 => 2,
        2 => 1,
        _ => 0,
    }
}

fn draw_legs(r: &mut Raster, ox: i32, legs: Legs, pal: &Palette) {
    let paw = |r: &mut Raster, x: i32, y: i32| {
        r.blob(x, y, 2, 1, pal.belly, pal.outline);
    };
    match legs {
        Legs::Stand => {
            paw(r, ox - 4, FEET);
            paw(r, ox + 4, FEET);
        }
        Legs::Walk(ph) => {
            // Two paws swinging in antiphase, lifting at the extremes.
            let swing = SWING[(ph % 8) as usize];
            paw(r, ox - 4 + swing, FEET - lift_of(swing));
            paw(r, ox + 4 - swing, FEET - lift_of(-swing));
        }
        Legs::Tuck => {
            r.blob(ox, FEET, 6, 1, pal.belly, pal.outline);
        }
        Legs::Splay => {
            paw(r, ox - 8, FEET - 4);
            paw(r, ox + 8, FEET - 4);
        }
        Legs::Dangle => {
            r.rect(ox - 5, FEET - 4, ox - 3, FEET - 1, pal.body);
            r.rect(ox + 3, FEET - 4, ox + 5, FEET - 1, pal.body);
            paw(r, ox - 4, FEET);
            paw(r, ox + 4, FEET);
        }
        Legs::Cling(ph) => {
            // Both limbs out on the wall ahead: one paw reaching for the lip,
            // the other pushing down by the hip, swapping through the cycle.
            //
            // Legs are drawn before the body, so anything inside the body's
            // silhouette (x within ox+-8, y within 16..28) is painted over and
            // vanishes. The reach has to clear it, and a short arm keeps the
            // paw attached rather than floating.
            let s = SWING[(ph % 8) as usize];
            let (hi, lo) = (10 + s, FEET - 4 - s);
            // Upper arm angles up to the lip; the lower one is nearly straight.
            r.rect(ox + 3, hi + 2, ox + 6, hi + 3, pal.body);
            r.rect(ox + 6, hi, ox + 9, hi + 1, pal.body);
            paw(r, ox + 10, hi);
            r.rect(ox + 3, lo, ox + 9, lo + 1, pal.body);
            paw(r, ox + 10, lo);
        }
    }
}

/// Zzz / steam / "!" — the default effect set, shared by any creature that
/// does not want something bespoke.
pub(crate) fn draw_fx(r: &mut Raster, p: &Pose, pal: &Palette) {
    draw_fx_at(r, p, pal, r.w / 2);
}

/// Same, for a creature whose head is not in the middle of its cell.
///
/// The positions below were chosen against a 32px frame with the head centred,
/// which is true of Pal, Vader and the mouse but not of the monkey — its head
/// sits well to the right of its hips, and effects laid out for the centre land
/// squarely on its face.
pub(crate) fn draw_fx_at(r: &mut Raster, p: &Pose, pal: &Palette, head_x: i32) {
    let width = r.w;
    let sx = move |x: i32| head_x + (x - CW / 2) * width / CW;
    match p.fx {
        Fx::None => {}
        Fx::Zzz(ph) => {
            // Three 'z's drifting up and to the right, each phase-shifted.
            // Sleeping is a lying pose with the head to the left, so they have
            // to start further left than they would above a standing creature.
            let base_x = if p.lying { 13 } else { 19 };
            for i in 0..3u8 {
                let t = ((ph + i * 2) % 6) as f32 / 6.0;
                let x = sx(base_x) + i as i32 * 4 + (t * 2.0) as i32;
                let y = 16 - i as i32 * 5 - (t * 3.0) as i32;
                let size = 3 - (i as i32).min(1);
                draw_z(r, x, y, size, fade(pal.accent, 1.0 - t * 0.7), pal.outline);
            }
        }
        Fx::Steam(ph) => {
            for i in 0..3u8 {
                let t = ((ph + i * 2) % 6) as f32 / 6.0;
                let x = sx(22) + i as i32 * 2;
                let y = 12 - i as i32 * 3 - (t * 3.0) as i32;
                r.ellipse(x, y, 2, 1, fade(argb(255, 0xE8, 0xE8, 0xF0), (1.0 - t) * 0.8));
            }
        }
        Fx::Bang(ph) => {
            // An outlined "!": a 3px-wide stem over a 1px dot. Anything
            // chunkier just reads as a white rectangle at this size.
            let bob = [0i32, -1, -2, -1][(ph % 4) as usize];
            let x = sx(26);
            let y = 4 + bob;
            r.rect(x - 2, y - 1, x + 2, y + 5, pal.outline);
            r.rect(x - 1, y, x + 1, y + 4, pal.accent);
            r.rect(x - 2, y + 6, x + 2, y + 8, pal.outline);
            r.put(x, y + 7, pal.accent);
        }
    }
}

pub(crate) fn draw_z(r: &mut Raster, x: i32, y: i32, s: i32, fill: u32, line: u32) {
    // Outline pass then fill, so the glyph stays readable on any wallpaper.
    for (c, o) in [(line, 1), (fill, 0)] {
        r.hline(x - o, x + s + o, y - o, c);
        r.hline(x - o, x + s + o, y + s + o, c);
        for i in 0..=s {
            r.put(x + s - i, y + i, c);
            if o == 1 {
                r.put(x + s - i + 1, y + i, c);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in creature
// ---------------------------------------------------------------------------

/// Which built-in creature to draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Pal,
    Vader,
    Mouse,
    Monkey,
}

impl Kind {
    /// The identifier used in `config.toml` and on the command line.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Pal => "pal",
            Kind::Vader => "vader",
            Kind::Mouse => "mouse",
            Kind::Monkey => "monkey",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Pal => "Pal (the blob)",
            Kind::Vader => "Darth Vader",
            Kind::Mouse => "Mouse",
            Kind::Monkey => "Monkey",
        }
    }

    pub fn from_id(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pal" | "" => Some(Kind::Pal),
            "vader" | "darth" | "darthvader" => Some(Kind::Vader),
            "mouse" | "mousey" => Some(Kind::Mouse),
            "monkey" | "ape" | "chimp" => Some(Kind::Monkey),
            _ => None,
        }
    }

    /// Frame size. The monkey needs the extra width: arms down, it is already
    /// as wide as it is tall, and the tail curls out well past the hip.
    pub fn frame_size(self) -> (u32, u32) {
        match self {
            Kind::Pal | Kind::Vader | Kind::Mouse => (CW as u32, CH as u32),
            Kind::Monkey => (crate::monkey::FW as u32, CH as u32),
        }
    }

    /// Half the creature's *body* as a fraction of frame width — what it uses
    /// to decide how close it can get to a ledge end. A trailing tail is not
    /// body, so the mouse's share is smaller than its frame suggests.
    pub fn body_half_frac(self) -> f32 {
        match self {
            Kind::Pal | Kind::Vader | Kind::Mouse => 0.375,
            // Of 36px, the torso and head span about 20 -- the rest is tail and
            // an outflung arm, neither of which needs to be over the ledge.
            Kind::Monkey => 0.28,
        }
    }

    /// Multiplier on the configured walking speed, so each creature moves the
    /// way it looks: the mouse skitters, Vader does not hurry.
    pub fn speed_scale(self) -> f32 {
        match self {
            Kind::Pal => 1.0,
            Kind::Vader => 0.8,
            Kind::Mouse => 1.7,
            // Quick and fidgety, but not a rodent sprint.
            Kind::Monkey => 1.25,
        }
    }

    /// Each creature needs its own colours: an all-black Sith lord under Pal's
    /// warm orange palette would be an orange Sith lord.
    pub fn palette(self) -> Palette {
        match self {
            Kind::Pal => Palette::default(),
            Kind::Vader => Palette {
                body: argb(255, 0x16, 0x17, 0x1C),
                belly: argb(255, 0x33, 0x38, 0x42),
                // A rim light rather than a dark outline: a black character
                // needs its silhouette drawn in something *lighter* than itself
                // or it vanishes against a dark taskbar.
                outline: argb(255, 0x7E, 0x86, 0x99),
                eye: argb(255, 0x8E, 0x1C, 0x22),
                blush: argb(255, 0xFF, 0x3B, 0x30), // saber red
                accent: argb(255, 0xE8, 0xEC, 0xF2),
            },
            Kind::Mouse => Palette {
                body: argb(255, 0x9C, 0x98, 0x93),
                belly: argb(255, 0xEA, 0xE3, 0xD8),
                outline: argb(255, 0x33, 0x2D, 0x2B),
                eye: argb(255, 0x18, 0x14, 0x13),
                // Ears, nose and tail.
                blush: argb(255, 0xE0, 0x9E, 0xA4),
                accent: argb(255, 0xFF, 0xFF, 0xFF),
            },
            Kind::Monkey => Palette {
                body: argb(255, 0x8A, 0x5A, 0x3B),
                // Face mask, chest and palms, all the same cream.
                belly: argb(255, 0xE8, 0xC9, 0xA0),
                outline: argb(255, 0x3A, 0x21, 0x18),
                eye: argb(255, 0x1A, 0x12, 0x0D),
                // Inner ears, muzzle and the tail's root.
                blush: argb(255, 0xC9, 0x8A, 0x72),
                accent: argb(255, 0xFF, 0xF3, 0xE0),
            },
        }
    }

    fn draw_fn(self) -> DrawFn {
        match self {
            Kind::Pal => draw_pose,
            Kind::Vader => crate::vader::draw,
            Kind::Mouse => crate::mouse::draw,
            Kind::Monkey => crate::monkey::draw,
        }
    }

    pub const ALL: [Kind; 4] = [Kind::Pal, Kind::Vader, Kind::Mouse, Kind::Monkey];
}

/// Renders one pose of a creature into a 32x32 frame.
pub(crate) type DrawFn = fn(&mut Raster, &Pose, &Palette);

/// Render a built-in creature into a fresh `SpriteSet`.
///
/// Everything is generated at start-up (a few hundred KB, ~1 ms) so the app has
/// no binary assets and users can recolour it from the config file.
pub fn builtin(kind: Kind, pal: &Palette) -> SpriteSet {
    let (w, h) = kind.frame_size();
    build_creature(kind.draw_fn(), pal, w as i32, h as i32)
}

/// The pose tables and frame timings, shared by every built-in creature — only
/// the `draw` function differs, so adding a creature is one drawing module.
fn build_creature(draw: DrawFn, pal: &Palette, fw: i32, fh: i32) -> SpriteSet {
    let mut frames: Vec<Frame> = Vec::new();
    let mut clips: Vec<Clip> = Vec::with_capacity(ANIM_COUNT);

    let bake = |frames: &mut Vec<Frame>, poses: &[Pose], frame_ms: u32| -> Clip {
        let mut idx = Vec::with_capacity(poses.len());
        for p in poses {
            let mut px = vec![0u32; (fw * fh) as usize];
            let mut r = Raster {
                px: &mut px,
                w: fw,
                h: fh,
            };
            draw(&mut r, p, pal);
            idx.push(frames.len() as u16);
            frames.push(Frame { px });
        }
        Clip { idx, frame_ms }
    };

    // Idle: a slow breath, a blink, and a glance around so a stationary pet
    // still looks alive rather than paused.
    let idle: Vec<Pose> = (0..8)
        .map(|i| Pose {
            squash: [0, 1, 1, 0, 0, 0, 1, 0][i],
            body_dy: [0, 0, 1, 0, 0, 0, 1, 0][i],
            head_dx: [0, 0, 0, 0, 1, 1, 1, 0][i],
            tail: [0, 1, 2, 1, 0, -1, -2, -1][i],
            eyes: if i == 3 { Eyes::Half } else { Eyes::Open },
            ..Default::default()
        })
        .collect();

    // Walk: an eight-phase gait. Four frames read as a shuffle; eight gives a
    // real stride with the body rising over the planted foot and a tail that
    // counter-swings through a full cycle.
    let walk: Vec<Pose> = (0..8)
        .map(|i| Pose {
            legs: Legs::Walk(i as u8),
            body_dy: [0, -1, -1, 0, 0, -1, -1, 0][i],
            head_dy: [0, -1, -1, 0, 0, -1, -1, 0][i],
            squash: [0, 0, -1, 0, 0, 0, -1, 0][i],
            tail: [0, 1, 2, 1, 0, -1, -2, -1][i],
            ..Default::default()
        })
        .collect();

    // Run: the same cycle stretched into a gallop — deeper bob, a stretch at
    // full extension, and the whole body leaning into it.
    let run: Vec<Pose> = (0..8)
        .map(|i| Pose {
            legs: Legs::Walk(i as u8),
            body_dy: [0, -2, -3, -1, 0, -2, -3, -1][i],
            head_dy: [1, -1, -2, 0, 1, -1, -2, 0][i],
            squash: [-1, 0, 1, 0, -1, 0, 1, 0][i],
            lean: [2, 3, 3, 2, 2, 3, 3, 2][i],
            head_dx: 2,
            ears_back: true,
            tail: [2, 2, 1, -1, -2, -2, -1, 1][i],
            eyes: Eyes::Wide,
            ..Default::default()
        })
        .collect();

    let fall: Vec<Pose> = (0..2)
        .map(|i| Pose {
            legs: Legs::Splay,
            eyes: Eyes::Wide,
            head_dy: -1,
            tail: [2, -2][i],
            squash: -1,
            ..Default::default()
        })
        .collect();

    let sleep: Vec<Pose> = (0..6)
        .map(|i| Pose {
            lying: true,
            eyes: Eyes::Closed,
            squash: [0, 0, 1, 1, 1, 0][i],
            body_dy: [0, 0, -1, -1, 0, 0][i],
            head_dy: [0, 0, -1, -1, 0, 0][i],
            ears_back: true,
            tail: 0,
            fx: Fx::Zzz(i as u8),
            ..Default::default()
        })
        .collect();

    let annoyed: Vec<Pose> = (0..4)
        .map(|i| Pose {
            eyes: Eyes::Angry,
            ears_back: true,
            head_dx: [-1, 1, -1, 1][i],
            lean: [1, -1, 1, -1][i],
            body_dy: [0, -1, 0, -1][i],
            tail: [2, -2, 2, -2][i],
            fx: Fx::Steam(i as u8),
            ..Default::default()
        })
        .collect();

    let drag: Vec<Pose> = (0..2)
        .map(|i| Pose {
            legs: Legs::Dangle,
            eyes: Eyes::Wide,
            squash: -1,
            head_dx: [-1, 1][i],
            tail: [1, -1][i],
            ..Default::default()
        })
        .collect();

    let alert: Vec<Pose> = (0..4)
        .map(|i| Pose {
            eyes: Eyes::Happy,
            body_dy: [0, -2, -1, 0][i],
            head_dy: [0, -1, 0, 0][i],
            blush: true,
            tail: [1, 2, 1, 0][i],
            fx: Fx::Bang(i as u8),
            ..Default::default()
        })
        .collect();

    let sit: Vec<Pose> = (0..2)
        .map(|i| Pose {
            legs: Legs::Tuck,
            body_dy: 2,
            squash: 1,
            eyes: Eyes::Half,
            tail: [0, 1][i],
            ..Default::default()
        })
        .collect();

    // Climb: hauling up the vertical edge of a window. The renderer cannot
    // rotate a cell, so the read has to come entirely from the pose — pressed
    // into the edge it is facing, head craned up at the lip, limbs reaching and
    // pushing in antiphase, and a body that heaves up on each pull rather than
    // rising smoothly. The tail hangs instead of counter-swinging: there is no
    // stride to balance.
    let climb: Vec<Pose> = (0..8)
        .map(|i| Pose {
            legs: Legs::Cling(i as u8),
            lean: [2, 3, 3, 2, 2, 3, 3, 2][i],
            body_dy: [0, -1, -2, -1, 0, -1, -2, -1][i],
            head_dy: [-2, -3, -3, -2, -2, -3, -3, -2][i],
            head_dx: 1,
            squash: [0, -1, -1, 0, 0, -1, -1, 0][i],
            ears_back: true,
            tail: [-2, -1, 0, -1, -2, -1, 0, -1][i],
            eyes: Eyes::Wide,
            ..Default::default()
        })
        .collect();

    // Order must match the `Anim` discriminants.
    clips.push(bake(&mut frames, &idle, 240));
    clips.push(bake(&mut frames, &walk, 65));
    clips.push(bake(&mut frames, &run, 38));
    clips.push(bake(&mut frames, &fall, 140));
    clips.push(bake(&mut frames, &sleep, 480));
    clips.push(bake(&mut frames, &annoyed, 110));
    clips.push(bake(&mut frames, &drag, 200));
    clips.push(bake(&mut frames, &alert, 150));
    clips.push(bake(&mut frames, &sit, 420));
    clips.push(bake(&mut frames, &climb, 85));

    SpriteSet {
        w: fw as u32,
        h: fh as u32,
        frames,
        clips,
    }
}

// ---------------------------------------------------------------------------
// User sprite sheets
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct SheetManifest {
    image: String,
    frame_width: u32,
    frame_height: u32,
    #[serde(default)]
    anims: std::collections::HashMap<String, AnimSpec>,
}

#[derive(serde::Deserialize)]
struct AnimSpec {
    /// Explicit frame indices (row-major over the sheet grid). Wins over `row`.
    #[serde(default)]
    frames: Vec<u16>,
    /// Shorthand: take `count` frames starting at the left of `row`.
    #[serde(default)]
    row: Option<u32>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default = "default_frame_ms")]
    frame_ms: u32,
}

fn default_frame_ms() -> u32 {
    120
}

/// Load `<dir>/sprite.toml` and its referenced PNG.
///
/// Any animation the manifest omits falls back to `idle`, so a sheet with a
/// single walk cycle is still a usable pet.
pub fn load_sheet(dir: &Path) -> Result<SpriteSet, String> {
    let manifest_path = dir.join("sprite.toml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let m: SheetManifest = toml::from_str(&text).map_err(|e| format!("sprite.toml: {e}"))?;

    if m.frame_width == 0 || m.frame_height == 0 {
        return Err("frame_width and frame_height must be non-zero".into());
    }

    let img_path = dir.join(&m.image);
    let (sheet, sw, sh) = decode_png(&img_path)?;

    let cols = sw / m.frame_width;
    let rows = sh / m.frame_height;
    if cols == 0 || rows == 0 {
        return Err(format!(
            "{} is {sw}x{sh}, smaller than one {}x{} frame",
            img_path.display(),
            m.frame_width,
            m.frame_height
        ));
    }
    let cells = (cols * rows) as u16;

    // Slice every cell once; clips then reference them by index.
    let fw = m.frame_width as usize;
    let fh = m.frame_height as usize;
    let mut frames = Vec::with_capacity(cells as usize);
    for cell in 0..cells as u32 {
        let cx = (cell % cols) as usize * fw;
        let cy = (cell / cols) as usize * fh;
        let mut px = vec![0u32; fw * fh];
        for y in 0..fh {
            let src = (cy + y) * sw as usize + cx;
            px[y * fw..(y + 1) * fw].copy_from_slice(&sheet[src..src + fw]);
        }
        frames.push(Frame { px });
    }

    let resolve = |spec: &AnimSpec| -> Vec<u16> {
        let mut idx: Vec<u16> = if !spec.frames.is_empty() {
            spec.frames.clone()
        } else if let Some(row) = spec.row {
            let n = spec.count.unwrap_or(cols).min(cols);
            (0..n).map(|i| (row * cols + i) as u16).collect()
        } else {
            Vec::new()
        };
        idx.retain(|&i| i < cells);
        idx
    };

    // `idle` is the fallback for everything else, so resolve it first and make
    // sure it is never empty.
    let idle_idx = m
        .anims
        .get("idle")
        .map(&resolve)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![0]);
    let idle_ms = m.anims.get("idle").map_or(200, |a| a.frame_ms.max(16));

    let mut clips = Vec::with_capacity(ANIM_COUNT);
    for a in Anim::ALL {
        match m.anims.get(a.key()) {
            Some(spec) => {
                let idx = resolve(spec);
                if idx.is_empty() {
                    clips.push(Clip {
                        idx: idle_idx.clone(),
                        frame_ms: idle_ms,
                    });
                } else {
                    clips.push(Clip {
                        idx,
                        frame_ms: spec.frame_ms.max(16),
                    });
                }
            }
            None => clips.push(Clip {
                idx: idle_idx.clone(),
                frame_ms: idle_ms,
            }),
        }
    }

    // `climb` is the newest animation, so most sheets in the wild predate it.
    // Idle is the wrong fallback here — a creature going up a window edge would
    // stand perfectly still while it moved. The walk cycle's moving legs read
    // fine against a vertical edge, which is what the built-ins used to do.
    if !m.anims.contains_key(Anim::Climb.key()) {
        let walk = &clips[Anim::Walk as usize];
        clips[Anim::Climb as usize] = Clip {
            idx: walk.idx.clone(),
            frame_ms: walk.frame_ms,
        };
    }

    Ok(SpriteSet {
        w: m.frame_width,
        h: m.frame_height,
        frames,
        clips,
    })
}

/// Decode a PNG to premultiplied BGRA.
fn decode_png(path: &Path) -> Result<(Vec<u32>, u32, u32), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width, info.height);

    let px: Vec<u32> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()]
            .chunks_exact(4)
            .map(|p| premultiply(argb(p[3], p[0], p[1], p[2])))
            .collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks_exact(3)
            .map(|p| argb(255, p[0], p[1], p[2]))
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .chunks_exact(2)
            .map(|p| premultiply(argb(p[1], p[0], p[0], p[0])))
            .collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .map(|&g| argb(255, g, g, g))
            .collect(),
        png::ColorType::Indexed => {
            return Err(format!("{}: unsupported indexed PNG", path.display()))
        }
    };

    if px.len() < (w * h) as usize {
        return Err(format!("{}: truncated image data", path.display()));
    }
    Ok((px, w, h))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Not an assertion — a developer aid. Composites every generated frame
    /// over a checkerboard and writes a contact sheet so the procedural art can
    /// be eyeballed. Run with `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_contact_sheet() {
        let kind = std::env::var("PETPAL_KIND")
            .ok()
            .and_then(|s| Kind::from_id(&s))
            .unwrap_or(Kind::Pal);
        let set = builtin(kind, &kind.palette());
        let (cw, ch) = (set.w as usize, set.h as usize);
        let per_row = 8usize;
        let rows = set.frames.len().div_ceil(per_row);
        let (sw, sh) = (per_row * cw, rows * ch);
        let mut out = vec![0u8; sw * sh * 3];

        for (n, f) in set.frames.iter().enumerate() {
            let (ox, oy) = ((n % per_row) * cw, (n / per_row) * ch);
            for y in 0..ch {
                for x in 0..cw {
                    let p = f.px[y * cw + x];
                    let a = (p >> 24) as u32;
                    // Premultiplied source over a checkerboard.
                    let bg = if ((x / 4) + (y / 4)) % 2 == 0 { 200 } else { 150 };
                    let ch_of = |sh: u32| {
                        (((p >> sh) & 0xff) + bg * (255 - a) / 255).min(255) as u8
                    };
                    let i = ((oy + y) * sw + ox + x) * 3;
                    out[i] = ch_of(0);
                    out[i + 1] = ch_of(8);
                    out[i + 2] = ch_of(16);
                }
            }
        }

        let path = std::env::var("PETPAL_DUMP")
            .unwrap_or_else(|_| std::env::temp_dir().join("petpal_sheet.bmp").display().to_string());
        write_bmp24(&path, &out, sw, sh);
        println!("wrote {path} ({sw}x{sh}, {} frames)", set.frames.len());
    }

    fn write_bmp24(path: &str, rgb: &[u8], w: usize, h: usize) {
        let stride = (w * 3 + 3) & !3;
        let size = 54 + stride * h;
        let mut b = Vec::with_capacity(size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        for y in (0..h).rev() {
            b.extend_from_slice(&rgb[y * w * 3..y * w * 3 + w * 3]);
            b.resize(b.len() + stride - w * 3, 0);
        }
        std::fs::write(path, b).unwrap();
    }
}
