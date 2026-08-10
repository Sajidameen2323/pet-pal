//! Fit an arbitrary contact sheet onto a PetPal sprite grid.
//!
//! Sheets that come out of an image generator are rarely usable as-is. They
//! tend to have some mix of:
//!
//!   * an opaque background instead of alpha,
//!   * sprites placed freehand, so no uniform cell grid exists,
//!   * a ground plate or shadow painted under each pose,
//!   * a render resolution far larger than the pixel art it depicts.
//!
//! This fixes all four and writes a `creature.png` + `sprite.toml` that PetPal
//! can load. Run with `--help` for the options.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// A pixel at or above this on every channel counts as background when keying.
const WHITE_LEVEL: u8 = 232;
/// Alpha above this counts as ink when measuring sprites.
const INK_ALPHA: u8 = 24;
/// Default per-channel tolerance for `--key-bg`.
const DEFAULT_BG_TOL: u8 = 30;

struct Img {
    w: usize,
    h: usize,
    px: Vec<[u8; 4]>, // straight RGBA
}

impl Img {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        self.px[y * self.w + x]
    }
    fn is_ink(&self, x: usize, y: usize) -> bool {
        self.px[y * self.w + x][3] > INK_ALPHA
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

struct Opts {
    input: PathBuf,
    out_dir: PathBuf,
    cell: usize,
    trim_bottom: usize,
    key_white: bool,
    key_bg: Option<u8>,
    cols: Option<usize>,
    rows: Option<usize>,
}

const USAGE: &str = "\
sheetconv - fit a contact sheet onto a PetPal sprite grid

USAGE:
    sheetconv <input.png> <output-dir> [options]

OPTIONS:
    --cell N           output cell size in pixels (default 64)
    --trim-bottom N    drop N pixels from the bottom of every sprite, in
                       source-image pixels. Use this when the art has a ground
                       plate or shadow painted under each pose -- that is
                       artwork, not background, so nothing else will remove it.
    --key-white        delete a WHITE background by flooding in from the image
                       borders. Only needed when the PNG has no alpha. Flooding
                       from the edge rather than keying by colour means white
                       *inside* the sprite (paws, chest) is kept.
    --key-bg [TOL]     same, but for a background of ANY colour, including a
                       gradient or a vignette. TOL is the largest per-channel
                       step it will follow (default 30). Raise it if a haze is
                       left behind; lower it if the creature loses its edges.
    --cols N           force the column count instead of detecting it
    --rows N           force the row count instead of detecting it
    -h, --help         this text

WHAT IT DOES
    Detects each sprite, scales them all by one shared factor so the creature
    does not pulse as it animates, and re-lays them on an exact grid with the
    feet on the bottom row of each cell and centred on the ink under the body
    (not the whole bounding box, so a swinging tail does not slide the sprite).

    Writes <output-dir>/creature.png and <output-dir>/sprite.toml. Copy that
    folder into %APPDATA%\\PetPal\\sprites\\ and pick it from Tray > Sprite.
";

fn parse_args() -> Result<Opts, String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.is_empty() || a.iter().any(|s| s == "-h" || s == "--help") {
        return Err(USAGE.into());
    }
    if a.len() < 2 {
        return Err("need <input.png> and <output-dir>; try --help".into());
    }
    let mut o = Opts {
        input: PathBuf::from(&a[0]),
        out_dir: PathBuf::from(&a[1]),
        cell: 64,
        trim_bottom: 0,
        key_white: false,
        key_bg: None,
        cols: None,
        rows: None,
    };
    let mut i = 2;
    while i < a.len() {
        let need = |i: usize| -> Result<usize, String> {
            a.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", a[i]))?
                .parse()
                .map_err(|_| format!("{} needs a number", a[i]))
        };
        match a[i].as_str() {
            "--cell" => { o.cell = need(i)?; i += 2 }
            "--trim-bottom" => { o.trim_bottom = need(i)?; i += 2 }
            "--cols" => { o.cols = Some(need(i)?); i += 2 }
            "--rows" => { o.rows = Some(need(i)?); i += 2 }
            "--key-white" => { o.key_white = true; i += 1 }
            "--key-bg" => {
                // Optional value: a bare --key-bg uses the default tolerance.
                match a.get(i + 1).and_then(|v| v.parse::<u8>().ok()) {
                    Some(t) => { o.key_bg = Some(t); i += 2 }
                    None => { o.key_bg = Some(DEFAULT_BG_TOL); i += 1 }
                }
            }
            other => return Err(format!("unknown option {other}; try --help")),
        }
    }
    if o.cell < 8 {
        return Err("--cell must be at least 8".into());
    }
    Ok(o)
}

// ---------------------------------------------------------------------------
// Image IO
// ---------------------------------------------------------------------------

fn read_png(path: &Path) -> Result<Img, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; r.output_buffer_size().unwrap_or(0)];
    let info = r.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];

    let px: Vec<[u8; 4]> = match info.color_type {
        png::ColorType::Rgb => data.chunks_exact(3).map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Rgba => data.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect(),
        png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => {
            data.chunks_exact(2).map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        other => return Err(format!("unsupported colour type {other:?}")),
    };
    Ok(Img { w, h, px })
}

fn write_png(path: &Path, px: &[[u8; 4]], w: usize, h: usize) -> Result<(), String> {
    let mut flat = Vec::with_capacity(w * h * 4);
    for p in px {
        flat.extend_from_slice(p);
    }
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&flat))
        .map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Background keying
// ---------------------------------------------------------------------------

/// Delete a background of any colour by flooding inward from the border.
///
/// `--key-white` only helps when the background really is white. Image
/// generators hand back grey studio backdrops, coloured washes and vignettes
/// just as often, and on those it does nothing at all — the whole sheet then
/// reads as one enormous sprite.
///
/// The test is against the *neighbour* rather than against a fixed seed colour,
/// so the flood walks a gradient: each step only has to be similar to the pixel
/// it spread from. Every border pixel is a seed, on the assumption that the art
/// does not run off the edge of its own sheet.
///
/// `tol` is the largest per-channel jump treated as "same background". Too high
/// and the flood eats into the creature wherever its colour approaches the
/// backdrop's.
fn key_background(img: &mut Img, tol: u8) {
    let (w, h) = (img.w, img.h);
    let near = |a: [u8; 4], b: [u8; 4]| {
        b[3] > INK_ALPHA
            && a[0].abs_diff(b[0]) <= tol
            && a[1].abs_diff(b[1]) <= tol
            && a[2].abs_diff(b[2]) <= tol
    };

    let mut bg = vec![false; w * h];
    let mut q: VecDeque<usize> = VecDeque::new();
    let seed = |i: usize, bg: &mut Vec<bool>, q: &mut VecDeque<usize>| {
        if !bg[i] && img.px[i][3] > INK_ALPHA {
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
        let mut n: Vec<usize> = Vec::with_capacity(4);
        if x > 0 { n.push(i - 1) }
        if x + 1 < w { n.push(i + 1) }
        if y > 0 { n.push(i - w) }
        if y + 1 < h { n.push(i + w) }
        for j in n {
            if !bg[j] && near(img.px[i], img.px[j]) {
                bg[j] = true;
                q.push_back(j);
            }
        }
    }
    for i in 0..w * h {
        if bg[i] {
            img.px[i] = [0, 0, 0, 0];
        }
    }
}

/// Delete the background by flooding inward from the border.
///
/// A plain colour key would punch holes in the artwork wherever the creature
/// itself is white — paws, chest, teeth. Only white that is *connected to the
/// edge* is background.
fn key_white_background(img: &mut Img) {
    let (w, h) = (img.w, img.h);
    let light = |p: [u8; 4]| {
        p[3] > INK_ALPHA && p[0] >= WHITE_LEVEL && p[1] >= WHITE_LEVEL && p[2] >= WHITE_LEVEL
    };
    let mut bg = vec![false; w * h];
    let mut q: VecDeque<usize> = VecDeque::new();

    for i in (0..w).chain((0..w).map(|x| (h - 1) * w + x)) {
        if light(img.px[i]) && !bg[i] {
            bg[i] = true;
            q.push_back(i);
        }
    }
    for y in 0..h {
        for i in [y * w, y * w + w - 1] {
            if light(img.px[i]) && !bg[i] {
                bg[i] = true;
                q.push_back(i);
            }
        }
    }
    while let Some(i) = q.pop_front() {
        let (x, y) = (i % w, i / w);
        let mut n: Vec<usize> = Vec::with_capacity(4);
        if x > 0 { n.push(i - 1) }
        if x + 1 < w { n.push(i + 1) }
        if y > 0 { n.push(i - w) }
        if y + 1 < h { n.push(i + w) }
        for j in n {
            if !bg[j] && light(img.px[j]) {
                bg[j] = true;
                q.push_back(j);
            }
        }
    }
    for i in 0..w * h {
        if bg[i] {
            img.px[i] = [0, 0, 0, 0];
        }
    }
}

// ---------------------------------------------------------------------------
// Grid detection
// ---------------------------------------------------------------------------

/// Runs of consecutive indices with non-zero occupancy, ignoring specks.
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

/// Split an axis into `n` equal slices, for when detection is overridden.
fn even_bands(len: usize, n: usize) -> Vec<(usize, usize)> {
    (0..n).map(|i| (i * len / n, (i + 1) * len / n - 1)).collect()
}

// ---------------------------------------------------------------------------
// Resampling
// ---------------------------------------------------------------------------

/// Area-average downscale in premultiplied space, so edges do not pick up a
/// dark or white halo from transparent neighbours.
fn resample(
    src: &Img,
    x0: usize, y0: usize, sw: usize, sh: usize,
    dw: usize, dh: usize,
) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; dw * dh];
    for dy in 0..dh {
        let sy0 = y0 + dy * sh / dh;
        let sy1 = (y0 + ((dy + 1) * sh).div_ceil(dh)).max(sy0 + 1).min(y0 + sh);
        for dx in 0..dw {
            let sx0 = x0 + dx * sw / dw;
            let sx1 = (x0 + ((dx + 1) * sw).div_ceil(dw)).max(sx0 + 1).min(x0 + sw);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let p = src.at(x, y);
                    let pa = p[3] as u64;
                    r += p[0] as u64 * pa;
                    g += p[1] as u64 * pa;
                    b += p[2] as u64 * pa;
                    a += pa;
                    n += 1;
                }
            }
            if n == 0 || a == 0 {
                continue;
            }
            out[dy * dw + dx] = [(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8];
        }
    }
    out
}

// ---------------------------------------------------------------------------

struct Sprite {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    /// Horizontal centre of the ink under the body, used as the anchor.
    anchor_x: usize,
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(if msg.starts_with("sheetconv") { 0 } else { 2 });
        }
    };
    if let Err(e) = run(&opts) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(o: &Opts) -> Result<(), String> {
    let mut img = read_png(&o.input)?;
    println!("input      {}x{}", img.w, img.h);

    if let Some(tol) = o.key_bg {
        key_background(&mut img, tol);
        println!("keyed      background removed by border flood, tolerance {tol}");
    } else if o.key_white {
        key_white_background(&mut img);
        println!("keyed      white background removed by border flood");
    }

    // Project ink onto each axis to find the sprite rows and columns.
    let (w, h) = (img.w, img.h);
    let (mut colocc, mut rowocc) = (vec![0usize; w], vec![0usize; h]);
    for y in 0..h {
        for x in 0..w {
            if img.is_ink(x, y) {
                colocc[x] += 1;
                rowocc[y] += 1;
            }
        }
    }
    let cols = match o.cols {
        Some(n) => even_bands(w, n),
        None => bands(&colocc, 8),
    };
    let rows = match o.rows {
        Some(n) => even_bands(h, n),
        None => bands(&rowocc, 8),
    };
    if cols.is_empty() || rows.is_empty() {
        return Err("no sprites found — is the image blank, or does it need --key-white?".into());
    }
    println!("grid       {} columns x {} rows = {} cells", cols.len(), rows.len(), cols.len() * rows.len());

    // Measure each sprite inside its own cell region.
    let mut sprites: Vec<Option<Sprite>> = Vec::new();
    for &(ry0, ry1) in &rows {
        for &(cx0, cx1) in &cols {
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for y in ry0..=ry1 {
                for x in cx0..=cx1 {
                    if img.is_ink(x, y) {
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
            // Drop the ground plate, if the sheet has one.
            y1 = y1.saturating_sub(o.trim_bottom).max(y0);

            // Anchor on the ink in the lowest fifth: the feet stay put between
            // frames, a tail or a raised paw does not.
            let foot_top = y1 - ((y1 - y0) / 5).max(1).min(y1 - y0);
            let (mut sum, mut cnt) = (0usize, 0usize);
            for y in foot_top..=y1 {
                for x in x0..=x1 {
                    if img.is_ink(x, y) {
                        sum += x;
                        cnt += 1;
                    }
                }
            }
            let anchor_x = if cnt > 0 { sum / cnt } else { (x0 + x1) / 2 };
            sprites.push(Some(Sprite { x0, y0, x1, y1, anchor_x }));
        }
    }

    let found = sprites.iter().flatten().count();
    if o.trim_bottom > 0 {
        println!("trim       {} px off the bottom of each sprite", o.trim_bottom);
    }

    // One scale for every frame. Scaling each sprite to fit its own box would
    // make the creature pulse as it animates.
    let (mut maxw, mut maxh) = (1usize, 1usize);
    for s in sprites.iter().flatten() {
        maxw = maxw.max(s.x1 - s.x0 + 1);
        maxh = maxh.max(s.y1 - s.y0 + 1);
    }
    let budget = o.cell as f64 * 0.97;
    let scale = (budget / maxw as f64).min(budget / maxh as f64);
    println!("sprites    {found} found, largest {maxw}x{maxh}, scaled by {scale:.3}");

    let (gc, gr) = (cols.len(), rows.len());
    let (sw, sh) = (gc * o.cell, gr * o.cell);
    let mut sheet = vec![[0u8; 4]; sw * sh];

    for (n, s) in sprites.iter().enumerate() {
        let Some(s) = s else { continue };
        let (bw, bh) = (s.x1 - s.x0 + 1, s.y1 - s.y0 + 1);
        let dw = ((bw as f64 * scale).round() as usize).clamp(1, o.cell);
        let dh = ((bh as f64 * scale).round() as usize).clamp(1, o.cell);
        let small = resample(&img, s.x0, s.y0, bw, bh, dw, dh);

        // Feet on the bottom row, anchor on the cell centre: exactly the
        // placement PetPal assumes when it stands a creature on a ledge.
        let (cell_x, cell_y) = (n % gc * o.cell, n / gc * o.cell);
        let anchor_off = ((s.anchor_x - s.x0) as f64 * scale).round() as isize;
        let ox = cell_x as isize + o.cell as isize / 2 - anchor_off;
        let oy = (cell_y + o.cell - dh) as isize;

        for y in 0..dh {
            for x in 0..dw {
                let p = small[y * dw + x];
                if p[3] == 0 {
                    continue;
                }
                let (tx, ty) = (ox + x as isize, oy + y as isize);
                // Clip to this cell so a wide pose cannot bleed into its neighbour.
                if tx < cell_x as isize || tx >= (cell_x + o.cell) as isize || ty < 0 || ty >= sh as isize {
                    continue;
                }
                sheet[ty as usize * sw + tx as usize] = p;
            }
        }
    }

    std::fs::create_dir_all(&o.out_dir).map_err(|e| format!("{}: {e}", o.out_dir.display()))?;
    write_png(&o.out_dir.join("creature.png"), &sheet, sw, sh)?;
    std::fs::write(o.out_dir.join("sprite.toml"), manifest(gc, gr, o.cell))
        .map_err(|e| format!("{}: {e}", o.out_dir.display()))?;

    println!("wrote      {}\\creature.png  ({sw}x{sh}, {}px cells)", o.out_dir.display(), o.cell);
    println!("wrote      {}\\sprite.toml", o.out_dir.display());
    Ok(())
}

/// A manifest for the produced grid.
///
/// Rows 0-2 are assumed to be idle / walk / run, which is how these sheets are
/// almost always laid out. Everything else is listed as comments with its frame
/// numbers so it can be assigned by hand — guessing which pose means "annoyed"
/// is not something a program should do silently.
fn manifest(cols: usize, rows: usize, cell: usize) -> String {
    let row_frames = |r: usize| -> String {
        (0..cols).map(|c| (r * cols + c).to_string()).collect::<Vec<_>>().join(", ")
    };
    let mut s = String::new();
    s.push_str("# Generated by tools/sheetconv.\n#\n");
    s.push_str("# Frames are numbered left to right, top to bottom from 0.\n");
    s.push_str("# Rows present in this sheet:\n");
    for r in 0..rows {
        s.push_str(&format!("#   row {r}: frames {}\n", row_frames(r)));
    }
    s.push_str(
        "#\n# Recognised animations: idle walk run fall sleep annoyed drag alert sit\n\
         # Anything omitted falls back to idle. See docs/SPRITES.md.\n\n",
    );
    s.push_str("image = \"creature.png\"\n");
    s.push_str(&format!("frame_width = {cell}\n"));
    s.push_str(&format!("frame_height = {cell}\n"));

    for (name, r, ms) in [("idle", 0usize, 200u32), ("walk", 1, 90), ("run", 2, 55)] {
        if r < rows {
            s.push_str(&format!("\n[anims.{name}]\nframes = [{}]\nframe_ms = {ms}\n", row_frames(r)));
        }
    }
    if rows > 3 {
        s.push_str("\n# Remaining frames, to assign by hand:\n");
        for r in 3..rows {
            s.push_str(&format!("#   row {r}: {}\n", row_frames(r)));
        }
        s.push_str(
            "#\n# for example:\n\
             # [anims.sleep]\n\
             # frames = [27, 28, 29, 30, 31]\n\
             # frame_ms = 480\n",
        );
    }
    s
}
