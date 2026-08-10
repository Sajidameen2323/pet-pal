//! Fitting an arbitrary contact sheet onto an exact sprite grid.
//!
//! A sheet out of an image generator is a picture *of* a sprite sheet rather
//! than a sprite sheet: the poses are placed by eye, there are wide margins,
//! each one often sits on a little ground plate, and the whole thing is
//! rendered several times larger than the pixel art it depicts. Slicing it on
//! any fixed cell size cuts creatures in half.
//!
//! So: find where the poses actually are, scale them all by **one** factor (a
//! per-sprite fit would make the creature pulse as it animates), and re-lay them
//! on a real grid with the feet on the bottom row of each cell — which is
//! exactly the placement the renderer assumes when it stands a creature on a
//! ledge.
//!
//! Deliberately self-contained: `tools/sheetconv` includes this same file so
//! the command line and the in-app editor cannot drift apart.

/// Straight RGBA packed as `0xAARRGGBB`.
pub type Px = u32;

/// Alpha above this counts as ink.
const INK: u32 = 24;
/// Default per-channel tolerance when keying a background.
pub const DEFAULT_BG_TOL: u8 = 30;
/// A band narrower than this is a speck, not a sprite.
const MIN_BAND: usize = 8;

#[inline]
fn alpha(p: Px) -> u32 {
    p >> 24
}

pub struct Options {
    /// Output cell size in pixels.
    pub cell: u32,
    /// Source pixels to drop from the bottom of every sprite, for sheets drawn
    /// with a ground plate under each pose.
    pub trim_bottom: u32,
    /// Remove an opaque background by flooding in from the border, with this
    /// per-channel tolerance.
    pub key_bg: Option<u8>,
    /// Force the grid instead of detecting it. Rarely wanted: forcing splits
    /// the *whole image* into equal bands, which is wrong the moment the sheet
    /// has margins.
    pub cols: Option<u32>,
    pub rows: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Options { cell: 64, trim_bottom: 0, key_bg: None, cols: None, rows: None }
    }
}

pub struct Fitted {
    pub px: Vec<Px>,
    pub w: u32,
    pub h: u32,
    pub cell: u32,
    pub cols: u32,
    pub rows: u32,
    /// How many cells actually got a sprite.
    pub found: u32,
    pub largest: (u32, u32),
    pub scale: f32,
}

/// Delete a background of any colour by flooding inward from the border.
///
/// The test is against the *neighbour* rather than a fixed seed colour, so the
/// flood walks a gradient: each step only has to resemble the pixel it spread
/// from. Every border pixel seeds it, on the assumption that the art does not
/// run off the edge of its own sheet. Flooding from the edge rather than keying
/// by colour is what keeps a white eye or a pale chest inside the creature.
pub fn key_background(px: &mut [Px], w: usize, h: usize, tol: u8) {
    if w == 0 || h == 0 {
        return;
    }
    let tol = tol as i32;
    let near = |a: Px, b: Px| {
        alpha(b) > INK
            && (((a >> 16) & 0xFF) as i32 - ((b >> 16) & 0xFF) as i32).abs() <= tol
            && (((a >> 8) & 0xFF) as i32 - ((b >> 8) & 0xFF) as i32).abs() <= tol
            && ((a & 0xFF) as i32 - (b & 0xFF) as i32).abs() <= tol
    };
    let mut bg = vec![false; w * h];
    let mut q: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let seed = |i: usize, bg: &mut Vec<bool>, q: &mut std::collections::VecDeque<usize>| {
        if !bg[i] && alpha(px[i]) > INK {
            bg[i] = true;
            q.push_back(i);
        }
    };
    for x in 0..w {
        seed(x, &mut bg, &mut q);
        seed((h - 1) * w + x, &mut bg, &mut q);
    }
    for y in 0..h {
        seed(y * w, &mut bg, &mut q);
        seed(y * w + w - 1, &mut bg, &mut q);
    }
    while let Some(i) = q.pop_front() {
        let (x, y) = (i % w, i / w);
        let mut n = [usize::MAX; 4];
        let mut c = 0;
        if x > 0 { n[c] = i - 1; c += 1 }
        if x + 1 < w { n[c] = i + 1; c += 1 }
        if y > 0 { n[c] = i - w; c += 1 }
        if y + 1 < h { n[c] = i + w; c += 1 }
        for &j in &n[..c] {
            if !bg[j] && near(px[i], px[j]) {
                bg[j] = true;
                q.push_back(j);
            }
        }
    }
    for i in 0..w * h {
        if bg[i] {
            px[i] = 0;
        }
    }
}

/// Runs of occupied lines, which is where the sprites are.
fn bands(occ: &[usize], min_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &v) in occ.iter().enumerate() {
        match (v > 0, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= min_len {
                    out.push((s, i - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if occ.len() - s >= min_len {
            out.push((s, occ.len() - 1));
        }
    }
    out
}

fn even_bands(len: usize, n: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| (i * len / n, (i + 1) * len / n - 1)).collect()
}

/// Detect the grid without doing any of the work, for callers that only want
/// to know whether a sheet needs fitting at all.
// Used by the in-app editor; the CLI includes this file too and does not.
#[allow(dead_code)]
pub fn detect_grid(px: &[Px], w: u32, h: u32) -> (u32, u32) {
    let (w, h) = (w as usize, h as usize);
    let (mut colocc, mut rowocc) = (vec![0usize; w], vec![0usize; h]);
    for y in 0..h {
        for x in 0..w {
            if alpha(px[y * w + x]) > INK {
                colocc[x] += 1;
                rowocc[y] += 1;
            }
        }
    }
    (
        bands(&colocc, MIN_BAND).len() as u32,
        bands(&rowocc, MIN_BAND).len() as u32,
    )
}

struct Sprite {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    /// Horizontal centre of the ink under the body. Anchoring on the whole
    /// bounding box would let a swinging tail slide the creature sideways.
    anchor_x: usize,
}

/// Area-average downscale in premultiplied space, so edges do not pick up a
/// dark or white halo from transparent neighbours.
fn resample(
    src: &[Px], sw_total: usize,
    x0: usize, y0: usize, sw: usize, sh: usize,
    dw: usize, dh: usize,
) -> Vec<Px> {
    let mut out = vec![0u32; dw * dh];
    for dy in 0..dh {
        let sy0 = y0 + dy * sh / dh;
        let sy1 = (y0 + ((dy + 1) * sh).div_ceil(dh)).max(sy0 + 1).min(y0 + sh);
        for dx in 0..dw {
            let sx0 = x0 + dx * sw / dw;
            let sx1 = (x0 + ((dx + 1) * sw).div_ceil(dw)).max(sx0 + 1).min(x0 + sw);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let p = src[y * sw_total + x];
                    let pa = alpha(p) as u64;
                    r += ((p >> 16) & 0xFF) as u64 * pa;
                    g += ((p >> 8) & 0xFF) as u64 * pa;
                    b += (p & 0xFF) as u64 * pa;
                    a += pa;
                    n += 1;
                }
            }
            if n == 0 || a == 0 {
                continue;
            }
            out[dy * dw + dx] =
                (((a / n) << 24) | ((r / a) << 16) | ((g / a) << 8) | (b / a)) as u32;
        }
    }
    out
}

/// Fit `src` onto an exact grid. Returns the new sheet, ready to slice.
pub fn fit(src: &[Px], w: u32, h: u32, o: &Options) -> Result<Fitted, String> {
    if w == 0 || h == 0 || src.len() < (w * h) as usize {
        return Err("empty image".into());
    }
    let cell = o.cell.max(8) as usize;
    let (iw, ih) = (w as usize, h as usize);

    let mut img: Vec<Px> = src.to_vec();
    if let Some(tol) = o.key_bg {
        key_background(&mut img, iw, ih, tol);
    }

    let (mut colocc, mut rowocc) = (vec![0usize; iw], vec![0usize; ih]);
    for y in 0..ih {
        for x in 0..iw {
            if alpha(img[y * iw + x]) > INK {
                colocc[x] += 1;
                rowocc[y] += 1;
            }
        }
    }
    let cols = match o.cols {
        Some(n) if n > 0 => even_bands(iw, n as usize),
        _ => bands(&colocc, MIN_BAND),
    };
    let rows = match o.rows {
        Some(n) if n > 0 => even_bands(ih, n as usize),
        _ => bands(&rowocc, MIN_BAND),
    };
    if cols.is_empty() || rows.is_empty() {
        return Err("no sprites found — the image is blank, or the background is opaque and needs removing".into());
    }
    if cols.len() * rows.len() > 1024 {
        return Err(format!(
            "detected {} x {} cells, which is too many to be a sprite sheet",
            cols.len(),
            rows.len()
        ));
    }

    let mut sprites: Vec<Option<Sprite>> = Vec::with_capacity(cols.len() * rows.len());
    for &(ry0, ry1) in &rows {
        for &(cx0, cx1) in &cols {
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for y in ry0..=ry1 {
                for x in cx0..=cx1 {
                    if alpha(img[y * iw + x]) > INK {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
            }
            if x0 == usize::MAX {
                sprites.push(None);
                continue;
            }
            y1 = y1.saturating_sub(o.trim_bottom as usize).max(y0);
            // Anchor on the ink in the lowest fifth: the feet stay put between
            // frames, a tail or a raised paw does not.
            let foot_top = y1 - ((y1 - y0) / 5).max(1).min(y1 - y0);
            let (mut sum, mut cnt) = (0usize, 0usize);
            for y in foot_top..=y1 {
                for x in x0..=x1 {
                    if alpha(img[y * iw + x]) > INK {
                        sum += x;
                        cnt += 1;
                    }
                }
            }
            let anchor_x = if cnt > 0 { sum / cnt } else { (x0 + x1) / 2 };
            sprites.push(Some(Sprite { x0, y0, x1, y1, anchor_x }));
        }
    }

    let found = sprites.iter().flatten().count() as u32;
    let (mut maxw, mut maxh) = (1usize, 1usize);
    for s in sprites.iter().flatten() {
        maxw = maxw.max(s.x1 - s.x0 + 1);
        maxh = maxh.max(s.y1 - s.y0 + 1);
    }
    let budget = cell as f64 * 0.97;
    let scale = (budget / maxw as f64).min(budget / maxh as f64);

    let (gc, gr) = (cols.len(), rows.len());
    let (sw, sh) = (gc * cell, gr * cell);
    let mut sheet = vec![0u32; sw * sh];

    for (n, s) in sprites.iter().enumerate() {
        let Some(s) = s else { continue };
        let (bw, bh) = (s.x1 - s.x0 + 1, s.y1 - s.y0 + 1);
        let dw = ((bw as f64 * scale).round() as usize).clamp(1, cell);
        let dh = ((bh as f64 * scale).round() as usize).clamp(1, cell);
        let small = resample(&img, iw, s.x0, s.y0, bw, bh, dw, dh);

        let (cell_x, cell_y) = (n % gc * cell, n / gc * cell);
        let anchor_off = ((s.anchor_x - s.x0) as f64 * scale).round() as isize;
        let ox = cell_x as isize + cell as isize / 2 - anchor_off;
        let oy = (cell_y + cell - dh) as isize;

        for y in 0..dh {
            for x in 0..dw {
                let p = small[y * dw + x];
                if alpha(p) == 0 {
                    continue;
                }
                let (tx, ty) = (ox + x as isize, oy + y as isize);
                // Clip to this cell so a wide pose cannot bleed into its neighbour.
                if tx < cell_x as isize
                    || tx >= (cell_x + cell) as isize
                    || ty < 0
                    || ty >= sh as isize
                {
                    continue;
                }
                sheet[ty as usize * sw + tx as usize] = p;
            }
        }
    }

    Ok(Fitted {
        px: sheet,
        w: sw as u32,
        h: sh as u32,
        cell: cell as u32,
        cols: gc as u32,
        rows: gr as u32,
        found,
        largest: (maxw as u32, maxh as u32),
        scale: scale as f32,
    })
}

/// Is this already a usable grid, or does it need fitting?
///
/// The distinction matters because fitting is destructive: it resamples and
/// re-anchors every pose. Doing that to a sheet somebody drew by hand on an
/// exact grid would soften their pixels for no reason. A sheet counts as ready
/// when a plausible cell size divides it exactly *and* every cell has something
/// in it — which is precisely what a hand-made or already-converted sheet looks
/// like, and precisely what generator output does not.
#[allow(dead_code)]
pub fn already_gridded(px: &[Px], w: u32, h: u32) -> Option<(u32, u32)> {
    for cell in [16u32, 24, 32, 48, 64, 96, 128] {
        if w % cell != 0 || h % cell != 0 {
            continue;
        }
        let (cols, rows) = (w / cell, h / cell);
        let n = cols * rows;
        if !(2..=400).contains(&n) {
            continue;
        }
        let all_inked = (0..rows).all(|r| {
            (0..cols).all(|c| {
                let (x0, y0) = (c * cell, r * cell);
                (0..cell).any(|y| {
                    (0..cell).any(|x| {
                        alpha(px[((y0 + y) * w + x0 + x) as usize]) > INK
                    })
                })
            })
        });
        if all_inked {
            return Some((cell, cell));
        }
    }
    None
}
