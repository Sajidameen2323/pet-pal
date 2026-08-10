//! Fit an arbitrary contact sheet onto a PetPal sprite grid, from the command
//! line.
//!
//! The app has the same thing built into **Tray > Sprite > Add a sprite...**,
//! which is easier to use because you can see the grid land. This exists for
//! batches and scripts.
//!
//! The fitting itself lives in the app's `src/regrid.rs` and is included here
//! rather than copied: two implementations of "where are the sprites" would
//! disagree eventually, and then the guide would be wrong about one of them.

use std::path::{Path, PathBuf};

#[path = "../../../src/regrid.rs"]
mod regrid;

struct Opts {
    input: PathBuf,
    out_dir: PathBuf,
    o: regrid::Options,
    key_white: bool,
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
    --key-bg [TOL]     delete a background of ANY colour, including a gradient
                       or a vignette, by flooding in from the image borders.
                       TOL is the largest per-channel step it will follow
                       (default 30). Raise it if a haze is left behind; lower it
                       if the creature loses its edges.
    --key-white        the older, narrower version: near-white backgrounds only.
    --cols N           force the column count instead of detecting it
    --rows N           force the row count instead of detecting it
    -h, --help         this text

    Forcing --cols/--rows splits the whole image into equal bands, which is
    wrong as soon as the sheet has margins around the art. Try it without them
    first.

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
    let mut opts = Opts {
        input: PathBuf::from(&a[0]),
        out_dir: PathBuf::from(&a[1]),
        o: regrid::Options::default(),
        key_white: false,
    };
    let mut i = 2;
    while i < a.len() {
        let need = |i: usize| -> Result<u32, String> {
            a.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", a[i]))?
                .parse()
                .map_err(|_| format!("{} needs a number", a[i]))
        };
        match a[i].as_str() {
            "--cell" => { opts.o.cell = need(i)?; i += 2 }
            "--trim-bottom" => { opts.o.trim_bottom = need(i)?; i += 2 }
            "--cols" => { opts.o.cols = Some(need(i)?); i += 2 }
            "--rows" => { opts.o.rows = Some(need(i)?); i += 2 }
            "--key-white" => { opts.key_white = true; i += 1 }
            "--key-bg" => {
                // Optional value: a bare --key-bg uses the default tolerance.
                match a.get(i + 1).and_then(|v| v.parse::<u8>().ok()) {
                    Some(t) => { opts.o.key_bg = Some(t); i += 2 }
                    None => { opts.o.key_bg = Some(regrid::DEFAULT_BG_TOL); i += 1 }
                }
            }
            other => return Err(format!("unknown option {other}; try --help")),
        }
    }
    if opts.o.cell < 8 {
        return Err("--cell must be at least 8".into());
    }
    // The old flag, in the new terms: white is just a very pale background.
    if opts.key_white && opts.o.key_bg.is_none() {
        opts.o.key_bg = Some(12);
    }
    Ok(opts)
}

fn read_png(path: &Path) -> Result<(Vec<u32>, u32, u32), String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; r.output_buffer_size().unwrap_or(0)];
    let info = r.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width, info.height);
    let data = &buf[..info.buffer_size()];
    let px: Vec<u32> = match info.color_type {
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .map(|p| 0xFF00_0000 | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
            .collect(),
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .map(|p| {
                ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
            })
            .collect(),
        png::ColorType::Grayscale => data
            .iter()
            .map(|&g| 0xFF00_0000 | ((g as u32) << 16) | ((g as u32) << 8) | g as u32)
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .map(|p| {
                ((p[1] as u32) << 24)
                    | ((p[0] as u32) << 16)
                    | ((p[0] as u32) << 8)
                    | p[0] as u32
            })
            .collect(),
        other => return Err(format!("unsupported colour type {other:?}")),
    };
    Ok((px, w, h))
}

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
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&flat))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A manifest for the produced grid.
///
/// Rows 0-2 are assumed to be idle / walk / run, which is how these sheets are
/// almost always laid out. Everything else is listed as comments with its frame
/// numbers so it can be assigned by hand -- guessing which pose means "annoyed"
/// is not something a program should do silently. The in-app editor is the
/// pleasant way to do that part.
fn manifest(cols: u32, rows: u32, cell: u32) -> String {
    let row_frames = |r: u32| -> String {
        (0..cols)
            .map(|c| (r * cols + c).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = String::new();
    s.push_str("# Converted by sheetconv.\n");
    s.push_str("# Rows 0-2 are guessed as idle / walk / run. Everything else is\n");
    s.push_str("# listed below for you to assign -- or use Tray > Sprite > Add a\n");
    s.push_str("# sprite... which lets you click the cells instead.\n\n");
    s.push_str("image = \"creature.png\"\n");
    s.push_str(&format!("frame_width = {cell}\nframe_height = {cell}\n"));
    for (name, row, ms) in [("idle", 0u32, 200u32), ("walk", 1, 90), ("run", 2, 55)] {
        if row < rows {
            s.push_str(&format!("\n[anims.{name}]\nframes = [{}]\nframe_ms = {ms}\n", row_frames(row)));
        }
    }
    if rows > 3 {
        s.push_str("\n# Unassigned frames, by row:\n");
        for r in 3..rows {
            s.push_str(&format!("#   row {r}: {}\n", row_frames(r)));
        }
        s.push_str("# Animation names: fall sleep annoyed drag alert sit climb\n");
    }
    s
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
    let (px, w, h) = read_png(&o.input)?;
    println!("input      {w}x{h}");
    if let Some(tol) = o.o.key_bg {
        println!("keyed      background removed by border flood, tolerance {tol}");
    }

    let f = regrid::fit(&px, w, h, &o.o)?;
    println!(
        "grid       {} columns x {} rows = {} cells",
        f.cols,
        f.rows,
        f.cols * f.rows
    );
    if o.o.trim_bottom > 0 {
        println!("trim       {} px off the bottom of each sprite", o.o.trim_bottom);
    }
    println!(
        "sprites    {} found, largest {}x{}, scaled by {:.3}",
        f.found, f.largest.0, f.largest.1, f.scale
    );

    std::fs::create_dir_all(&o.out_dir).map_err(|e| format!("{}: {e}", o.out_dir.display()))?;
    write_png(&o.out_dir.join("creature.png"), &f.px, f.w, f.h)?;
    std::fs::write(o.out_dir.join("sprite.toml"), manifest(f.cols, f.rows, f.cell))
        .map_err(|e| format!("{}: {e}", o.out_dir.display()))?;
    println!(
        "wrote      {}\\creature.png  ({}x{}, {}px cells)",
        o.out_dir.display(),
        f.w,
        f.h,
        f.cell
    );
    println!("wrote      {}\\sprite.toml", o.out_dir.display());
    Ok(())
}
