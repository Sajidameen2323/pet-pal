//! Getting pixels *out* of the app: exporting a creature as an editable sprite
//! sheet, and (test-only) building the multi-resolution `.ico` for the exe.
//!
//! Both go the opposite way from [`crate::sprites::load_sheet`], and both have
//! to undo the premultiplied alpha the renderer works in — PNG and the ICO
//! bitmap format both store straight alpha.

use std::path::Path;

use crate::sprites::{Anim, SpriteSet};

/// Columns in an exported sheet. Matches the layout the loader documents.
const SHEET_COLS: u32 = 8;

/// Undo premultiplication and swizzle BGRA to RGBA, which is what both PNG and
/// icon bitmaps expect.
fn straight_rgba(px: &[u32], out: &mut Vec<u8>) {
    for &p in px {
        let a = (p >> 24) & 0xff;
        let (r, g, b) = if a == 0 || a == 255 {
            ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff)
        } else {
            let un = |c: u32| ((c * 255) / a).min(255);
            (un((p >> 16) & 0xff), un((p >> 8) & 0xff), un(p & 0xff))
        };
        out.extend_from_slice(&[r as u8, g as u8, b as u8, a as u8]);
    }
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Sprite sheet export
// ---------------------------------------------------------------------------

/// Write `set` to `dir` as `creature.png` plus a `sprite.toml` that reproduces
/// it exactly — the same frame order and timings the built-in uses.
///
/// This is the starting point for a custom creature: export, repaint the PNG,
/// pick the folder from the tray. Because the manifest lists real frame
/// indices rather than rows, an edited sheet keeps every animation intact even
/// if the author only touches a few cells.
pub fn export_sheet(set: &SpriteSet, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let cols = SHEET_COLS;
    let rows = (set.frames.len() as u32).div_ceil(cols);
    let (fw, fh) = (set.w, set.h);
    let (sw, sh) = (cols * fw, rows * fh);

    // Compose the grid in premultiplied form, then convert once.
    let mut grid = vec![0u32; (sw * sh) as usize];
    for (n, frame) in set.frames.iter().enumerate() {
        let (cx, cy) = (n as u32 % cols * fw, n as u32 / cols * fh);
        for y in 0..fh {
            let src = (y * fw) as usize;
            let dst = ((cy + y) * sw + cx) as usize;
            grid[dst..dst + fw as usize]
                .copy_from_slice(&frame.px[src..src + fw as usize]);
        }
    }

    let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
    straight_rgba(&grid, &mut rgba);
    let png = encode_png(&rgba, sw, sh)?;
    std::fs::write(dir.join("creature.png"), png)
        .map_err(|e| format!("{}: {e}", dir.display()))?;

    std::fs::write(dir.join("sprite.toml"), manifest_for(set))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(())
}

/// A manifest that reproduces `set` frame for frame.
fn manifest_for(set: &SpriteSet) -> String {
    let mut s = String::from(
        "# Exported from PetPal. Repaint creature.png and keep this file next\n\
         # to it; the folder then shows up under Tray > Sprite.\n\
         #\n\
         # Frames are numbered left-to-right, top-to-bottom from 0. Draw facing\n\
         # RIGHT (PetPal mirrors for leftward movement) and keep the feet on the\n\
         # bottom row of each cell -- that row is aligned with the ground.\n\n",
    );
    s.push_str(&format!("image = \"creature.png\"\n"));
    s.push_str(&format!("frame_width = {}\n", set.w));
    s.push_str(&format!("frame_height = {}\n", set.h));

    for a in Anim::ALL {
        let clip = set.clip(a);
        let list: Vec<String> = clip.idx.iter().map(|i| i.to_string()).collect();
        s.push_str(&format!("\n[anims.{}]\n", a.key()));
        s.push_str(&format!("frames = [{}]\n", list.join(", ")));
        s.push_str(&format!("frame_ms = {}\n", clip.frame_ms));
    }
    s
}

// ---------------------------------------------------------------------------
// Application icon
// ---------------------------------------------------------------------------

/// Tight bounds of the non-transparent pixels in a frame.
#[cfg(test)]
fn opaque_bounds(px: &[u32], w: u32, h: u32) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) as usize] >> 24 != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 {
        (0, 0, w - 1, h - 1)
    } else {
        (x0, y0, x1, y1)
    }
}

/// Box-resample a straight-alpha RGBA image into `dw` x `dh`.
///
/// One code path covers both directions: when scaling up, the source rectangle
/// for a destination pixel collapses to a single texel, which is exactly
/// nearest-neighbour — so pixel art stays crisp instead of going soft.
#[cfg(test)]
fn resample(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = (((dy + 1) * sh).div_ceil(dh)).max(y0 + 1).min(sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = (((dx + 1) * sw).div_ceil(dw)).max(x0 + 1).min(sw);

            // Average colour weighted by alpha, so transparent pixels do not
            // drag the edges toward black.
            let (mut ar, mut ag, mut ab, mut aa, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * sw + x) * 4) as usize;
                    let a = src[i + 3] as u32;
                    ar += src[i] as u32 * a;
                    ag += src[i + 1] as u32 * a;
                    ab += src[i + 2] as u32 * a;
                    aa += a;
                    n += 1;
                }
            }
            let o = ((dy * dw + dx) * 4) as usize;
            if aa > 0 {
                out[o] = (ar / aa) as u8;
                out[o + 1] = (ag / aa) as u8;
                out[o + 2] = (ab / aa) as u8;
                out[o + 3] = (aa / n.max(1)) as u8;
            }
        }
    }
    out
}

/// Fit `src` into a `size` x `size` canvas, centred, with a little breathing room.
#[cfg(test)]
fn fit_square(src: &[u8], sw: u32, sh: u32, size: u32) -> Vec<u8> {
    let budget = (size as f32 * 0.92).max(1.0);
    let scale = (budget / sw.max(sh) as f32).max(0.01);
    // Whole-number scales above 1:1 keep the pixel grid intact.
    let scale = if scale >= 1.0 { scale.floor() } else { scale };
    let dw = ((sw as f32 * scale).round() as u32).clamp(1, size);
    let dh = ((sh as f32 * scale).round() as u32).clamp(1, size);

    let small = resample(src, sw, sh, dw, dh);
    let mut out = vec![0u8; (size * size * 4) as usize];
    let (ox, oy) = ((size - dw) / 2, (size - dh) / 2);
    for y in 0..dh {
        let s = ((y * dw) * 4) as usize;
        let d = (((oy + y) * size + ox) * 4) as usize;
        out[d..d + (dw * 4) as usize].copy_from_slice(&small[s..s + (dw * 4) as usize]);
    }
    out
}

/// A 32bpp BMP icon image: header, bottom-up BGRA, then an empty AND mask.
///
/// The AND mask is redundant for 32bpp images but the format still requires it,
/// and some shell surfaces reject entries that omit it.
#[cfg(test)]
fn ico_bmp(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // XOR + AND
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&[0u8; 24]); // compression .. clrImportant

    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = ((size + 31) / 32) * 4;
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

/// Build a multi-resolution `.ico` from a creature's idle frame.
///
/// Only ever called by the `generate_app_icon` test: the result is committed as
/// `assets/petpal.ico` and embedded at build time, so the shipped binary has no
/// reason to carry an icon encoder.
#[cfg(test)]
pub fn build_icon(set: &SpriteSet) -> Result<Vec<u8>, String> {
    let frame = set.frame(Anim::Idle, 0);
    let (x0, y0, x1, y1) = opaque_bounds(&frame.px, set.w, set.h);
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);

    // Crop to the creature so it fills the icon rather than floating in the
    // frame's empty margins.
    let mut cropped = Vec::with_capacity((cw * ch) as usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            cropped.push(frame.px[(y * set.w + x) as usize]);
        }
    }
    let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
    straight_rgba(&cropped, &mut rgba);

    // Everyday sizes go in as BMP: the shell reads PNG entries fine, but plenty
    // of tooling (System.Drawing.Icon among it) still cannot, and these are the
    // sizes anything is likely to ask for. The two big ones are PNG so the file
    // does not balloon — a 256px BMP entry alone is 256 KB.
    const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    const PNG_FROM: u32 = 128;
    let mut images: Vec<(u32, Vec<u8>)> = Vec::with_capacity(SIZES.len());
    for size in SIZES {
        let fitted = fit_square(&rgba, cw, ch, size);
        let data = if size >= PNG_FROM {
            encode_png(&fitted, size, size)?
        } else {
            ico_bmp(&fitted, size)
        };
        images.push((size, data));
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        // 256 is encoded as 0 in a single byte.
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0); // palette entries
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprites::{builtin, Kind, ANIM_COUNT};

    /// Dump every built-in as a sheet under `target/preview/` so a change to
    /// the pose tables can actually be looked at. Developer tooling:
    /// `cargo test -- --ignored preview_builtin_sheets`.
    #[test]
    #[ignore]
    fn preview_builtin_sheets() {
        for kind in Kind::ALL {
            let set = builtin(kind, &kind.palette());
            let dir = std::path::Path::new("target/preview").join(kind.id());
            export_sheet(&set, &dir).expect("export");
            println!("{} -> {}", kind.label(), dir.display());
        }
    }

    /// Regenerate `assets/petpal.ico`, which `build.rs` embeds in the exe.
    /// Developer tooling, not an assertion: run with
    /// `cargo test -- --ignored generate_app_icon`.
    #[test]
    #[ignore]
    fn generate_app_icon() {
        let kind = Kind::Pal;
        let set = builtin(kind, &kind.palette());
        let ico = build_icon(&set).expect("icon");
        std::fs::create_dir_all("assets").unwrap();
        std::fs::write("assets/petpal.ico", &ico).unwrap();
        println!("wrote assets/petpal.ico ({} bytes)", ico.len());
    }

    /// Pull every ```toml block out of the shipped guide.
    fn guide_toml_blocks() -> Vec<String> {
        crate::config::SPRITE_GUIDE
            .split("```toml")
            .skip(1)
            .filter_map(|rest| rest.split_once("```").map(|(body, _)| body.to_string()))
            .collect()
    }

    /// Write a blank RGBA sheet of `cols x rows` cells, plus `manifest`, and
    /// load it back the way the app would.
    fn load_manifest(manifest: &str, cell: u32, cols: u32, rows: u32, tag: &str) -> SpriteSet {
        let dir = std::env::temp_dir().join(format!("petpal-guide-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (cols * cell, rows * cell);
        let rgba = vec![0xFFu8; (w * h * 4) as usize];
        std::fs::write(dir.join("creature.png"), encode_png(&rgba, w, h).unwrap()).unwrap();
        std::fs::write(dir.join("sprite.toml"), manifest).unwrap();
        let set = crate::sprites::load_sheet(&dir)
            .unwrap_or_else(|e| panic!("guide manifest {tag} does not load: {e}"));
        let _ = std::fs::remove_dir_all(&dir);
        set
    }

    /// The guide is what users hand to an image generator and then copy
    /// verbatim, so a manifest printed in it that does not actually work is
    /// worse than no manifest at all — they have no way to tell the difference
    /// between a bad sheet and bad instructions.
    #[test]
    fn the_guide_manifests_are_real() {
        let blocks = guide_toml_blocks();
        assert!(blocks.len() >= 3, "expected the guide to print several manifests");
        for (i, b) in blocks.iter().enumerate() {
            toml::from_str::<toml::Table>(b)
                .unwrap_or_else(|e| panic!("guide toml block {i} is not valid TOML: {e}"));
        }

        // The beginner layout: an 8x7 sheet, 56 squares, described cell by cell
        // in the prompt users paste into an image generator. The manifest and
        // that description have to agree, and the only way they can is if the
        // 56 squares are partitioned — every square claimed, by exactly one
        // animation. A gap means the user drew a pose nothing plays; an overlap
        // means two animations show the same art.
        let simple = blocks
            .iter()
            .find(|b| b.contains("frame_width = 64"))
            .expect("guide should print the 8x7 beginner manifest");
        let set = load_manifest(simple, 64, 8, 7, "simple");

        let expected = [
            (Anim::Idle, 8),
            (Anim::Walk, 8),
            (Anim::Run, 8),
            (Anim::Climb, 4),
            (Anim::Fall, 4),
            (Anim::Sleep, 4),
            (Anim::Annoyed, 4),
            (Anim::Alert, 4),
            (Anim::Sit, 2),
            (Anim::Drag, 2),
            (Anim::Play, 8),
        ];
        assert_eq!(expected.len(), ANIM_COUNT, "every animation must be listed");
        let mut claimed: Vec<u16> = Vec::new();
        for (a, n) in expected {
            assert_eq!(
                set.frame_count(a),
                n,
                "{a:?} should use {n} squares in the 8x7 layout"
            );
            claimed.extend_from_slice(&set.clip(a).idx);
        }
        claimed.sort_unstable();
        let unique = {
            let mut c = claimed.clone();
            c.dedup();
            c
        };
        assert_eq!(claimed.len(), unique.len(), "two animations share a square");
        assert_eq!(
            unique,
            (0..56u16).collect::<Vec<_>>(),
            "the manifest must claim all 56 squares of the 8x7 sheet exactly once"
        );

        // The built-in layout, which has to match what export actually writes.
        // Matched on the run row, which only the 60-frame layout spells out —
        // the 8x7 one uses `row = 2`. Matching on something both contain (both
        // have a `frames = [44,` line) silently tests the wrong block.
        let full = blocks
            .iter()
            .find(|b| b.contains("frames = [16, 17, 18"))
            .expect("guide should print the 60-frame built-in manifest");
        let set = load_manifest(full, 32, 8, 8, "full");
        let reference = builtin(Kind::Pal, &Kind::Pal.palette());
        for a in Anim::ALL {
            assert_eq!(
                set.frame_count(a),
                reference.frame_count(a),
                "{a:?} frame count in the guide does not match the built-in"
            );
            assert_eq!(
                set.clip(a).idx,
                reference.clip(a).idx,
                "{a:?} frame numbers in the guide do not match what export writes"
            );
        }
    }

    /// A clip must run at the speed it says, whatever the caller's tick rate.
    ///
    /// After T milliseconds a clip of `frame_ms` is on frame `(T / frame_ms) %
    /// n`, full stop — the caller's tick size must not enter into it. The
    /// editor's preview used to reset its accumulator instead of keeping the
    /// remainder, which rounds every frame up to a whole tick: on a 40ms timer
    /// a 55ms run frame played at 80ms and a 90ms walk at 120ms. The author
    /// would then pick a speed that looked right in the preview and get a
    /// different one on the desktop.
    #[test]
    fn a_clip_runs_at_its_stated_speed_on_any_tick() {
        use crate::sprites::step_clip;

        for &frame_ms in &[16u32, 38, 55, 65, 90, 120, 240, 480] {
            for &tick in &[1u32, 13, 16, 25, 40, 100] {
                for &n in &[2usize, 3, 8] {
                    let (mut frame, mut acc) = (0usize, 0u32);
                    let mut elapsed = 0u32;
                    // Five seconds is several loops of even the slowest clip.
                    while elapsed + tick <= 5_000 {
                        step_clip(&mut frame, &mut acc, tick, frame_ms, n);
                        elapsed += tick;
                        assert_eq!(
                            frame,
                            (elapsed / frame_ms) as usize % n,
                            "frame_ms {frame_ms}, tick {tick}, n {n}, at {elapsed}ms"
                        );
                    }
                }
            }
        }
    }

    /// The reported "changed" flag drives the repaint, so a missed one is a
    /// frame the viewer never sees and a spurious one is a wasted redraw.
    #[test]
    fn step_clip_reports_exactly_when_the_frame_changes() {
        use crate::sprites::step_clip;

        let (mut frame, mut acc) = (0usize, 0u32);
        // 100ms frames, 40ms ticks: changes at 100, 200, 300...
        let expect = [false, false, true, false, true, false, false, true];
        for (i, &want) in expect.iter().enumerate() {
            let before = frame;
            let got = step_clip(&mut frame, &mut acc, 40, 100, 4);
            assert_eq!(got, want, "tick {i}");
            assert_eq!(got, frame != before, "tick {i}: flag disagrees with the frame");
        }

        // A single frame never changes, however long you wait.
        let (mut f1, mut a1) = (0usize, 0u32);
        assert!(!step_clip(&mut f1, &mut a1, 5_000, 50, 1));
        assert_eq!(f1, 0);

        // An empty clip is not a crash.
        let (mut f0, mut a0) = (0usize, 0u32);
        assert!(!step_clip(&mut f0, &mut a0, 1_000, 50, 0));
    }

    /// Every built-in must actually draw every animation, and its climb has to
    /// be its own artwork rather than a rename of the walk cycle. Adding a
    /// creature is easy to half-finish: a `Legs` arm that falls through to the
    /// default renders something, so nothing crashes and nothing looks obviously
    /// broken until you watch it go up a window.
    #[test]
    fn every_builtin_animates_every_pose() {
        for kind in Kind::ALL {
            let set = builtin(kind, &kind.palette());
            let name = kind.id();

            for a in Anim::ALL {
                let clip = set.clip(a);
                assert!(!clip.idx.is_empty(), "{name} {a:?} has no frames");
                for i in 0..clip.idx.len() {
                    let ink = set.frame(a, i).px.iter().filter(|&&p| p >> 24 > 24).count();
                    assert!(ink > 40, "{name} {a:?}[{i}] is blank or near-blank");
                }
            }

            // A pose table that forgot to vary is the other half-finish: eight
            // identical cells still animate, they just do not move.
            for a in [Anim::Walk, Anim::Run, Anim::Climb, Anim::Play] {
                let first = &set.frame(a, 0).px;
                assert!(
                    (1..set.frame_count(a)).any(|i| &set.frame(a, i).px != first),
                    "{name} {a:?} is the same frame repeated"
                );
            }

            // Climbing must not just be walking under another name.
            let n = set.frame_count(Anim::Climb).min(set.frame_count(Anim::Walk));
            assert!(
                (0..n).any(|i| set.frame(Anim::Climb, i).px != set.frame(Anim::Walk, i).px),
                "{name} climb is the walk cycle"
            );

            // Nor may a trick be the alert reaction relabelled — that is what a
            // sheet *without* a trick falls back to, so a built-in doing the
            // same has effectively not got one.
            let n = set.frame_count(Anim::Play).min(set.frame_count(Anim::Alert));
            assert!(
                (0..n).any(|i| set.frame(Anim::Play, i).px != set.frame(Anim::Alert, i).px),
                "{name} play is the alert animation"
            );

            // The trick leaves the ground in the middle and is back on it at
            // both ends. Measured against how *this* creature stands rather
            // than against the bottom row of the cell: Vader and the monkey
            // draw their feet a pixel or two clear of it in every pose, so
            // "touches the last row" would be a claim about them and not about
            // the animation.
            let floor_of = |a: Anim, i: usize| -> u32 {
                let f = set.frame(a, i);
                let w = set.w as usize;
                (0..set.h)
                    .rev()
                    .find(|&y| {
                        f.px[y as usize * w..(y as usize + 1) * w]
                            .iter()
                            .any(|&p| p >> 24 > 24)
                    })
                    .unwrap_or(0)
            };
            let stand = floor_of(Anim::Idle, 0);
            let n = set.frame_count(Anim::Play);
            for i in [0, n - 1] {
                assert!(
                    floor_of(Anim::Play, i) + 3 >= stand,
                    "{name} play[{i}] should have its feet down, like idle does"
                );
            }
            // Strictly higher than the resting pose, not higher by some margin:
            // a creature whose limbs hang when splayed keeps its lowest pixel
            // almost where it was even at the top of a clear hop. The mouse
            // gains one pixel here where Pal gains four, and both read fine.
            let rest = floor_of(Anim::Play, 0);
            assert!(
                (1..n - 1).any(|i| floor_of(Anim::Play, i) < rest),
                "{name} play never leaves the ground"
            );
        }
    }

    /// Every sheet under `assets/sprites` must load, and no clip may point at a
    /// blank cell. A manifest that references an empty cell is not an error the
    /// loader can catch — the pet just turns invisible for that animation, which
    /// is exactly the bug the shipped rabbit manifest started life with.
    #[test]
    fn shipped_sheets_have_no_blank_frames() {
        let root = std::path::Path::new("assets/sprites");
        let mut checked = 0;
        for entry in std::fs::read_dir(root).expect("assets/sprites").flatten() {
            let dir = entry.path();
            if !dir.join("sprite.toml").is_file() {
                continue;
            }
            let name = entry.file_name();
            let set = crate::sprites::load_sheet(&dir)
                .unwrap_or_else(|e| panic!("{}: {e}", name.to_string_lossy()));
            for a in Anim::ALL {
                let clip = set.clip(a);
                assert!(!clip.idx.is_empty(), "{name:?} {a:?} has no frames");
                for (i, &cell) in clip.idx.iter().enumerate() {
                    let opaque = set.frames[cell as usize]
                        .px
                        .iter()
                        .filter(|&&p| p >> 24 > 24)
                        .count();
                    assert!(opaque > 0, "{name:?} {a:?}[{i}] is blank cell {cell}");
                }
            }
            checked += 1;
        }
        assert!(checked > 0, "no sheets found under {}", root.display());
    }

    /// A sheet with cells far larger than the editor's preview box has to load
    /// and preview like any other. Big art is the ordinary case for a sheet out
    /// of an image generator, and it used to be the case that killed the
    /// process: the preview box is a fixed 288x150, and nothing scaled a 256px
    /// cell down to fit it.
    #[test]
    fn a_sheet_with_cells_larger_than_the_preview_box_works_end_to_end() {
        for cell in [128u32, 192, 256, 384] {
            let (cols, rows) = (4u32, 2u32);
            let manifest = format!(
                "image = \"creature.png\"\n\
                 frame_width = {cell}\nframe_height = {cell}\n\
                 [anims.idle]\nframes = [0, 1, 2, 3]\nframe_ms = 200\n\
                 [anims.walk]\nframes = [4, 5, 6, 7]\nframe_ms = 100\n"
            );
            let tag = format!("big-{cell}");
            let set = load_manifest(&manifest, cell, cols, rows, &tag);
            assert_eq!(set.w, cell);
            assert_eq!(set.frames.len(), (cols * rows) as usize);
            assert_eq!(set.frame_count(Anim::Idle), 4);

            // And the editor can composite every one of those cells into its
            // preview box without going outside the buffer.
            let px: Vec<u32> = vec![0xFFFF_FFFF; (cols * cell * rows * cell) as usize];
            for n in 0..(cols * rows) as u16 {
                let buf = crate::addsprite::compose_play(
                    &px,
                    (cols * cell) as i32,
                    (rows * cell) as i32,
                    n,
                    cols as i32,
                    cell as i32,
                    cell as i32,
                    288,
                    150,
                );
                assert_eq!(buf.len(), 288 * 150, "{cell}px cell, frame {n}");
            }
        }
    }

    /// Every sheet written before `play` existed lacks one, and there are
    /// already sheets in the wild. Falling back to `idle` would mean the
    /// creature announced a trick and then stood perfectly still; `alert` is at
    /// least a pleased, moving reaction, which is the nearest thing such a
    /// sheet actually contains.
    #[test]
    fn a_sheet_without_a_trick_borrows_its_alert() {
        let manifest = "\
image = \"creature.png\"
frame_width = 32
frame_height = 32

[anims.idle]
frames = [0]
frame_ms = 200

[anims.alert]
frames = [1, 2, 3]
frame_ms = 150
";
        let set = load_manifest(manifest, 32, 4, 1, "no-play");
        assert_eq!(
            set.clip(Anim::Play).idx,
            vec![1, 2, 3],
            "play should borrow the alert frames, not idle's"
        );
        assert_eq!(set.clip(Anim::Play).frame_ms, 150, "and its timing");

        // A sheet that *does* declare one keeps its own.
        let with = format!("{manifest}\n[anims.play]\nframes = [3, 2]\nframe_ms = 90\n");
        let set = load_manifest(&with, 32, 4, 1, "with-play");
        assert_eq!(set.clip(Anim::Play).idx, vec![3, 2]);
        assert_eq!(set.clip(Anim::Play).frame_ms, 90);
    }

    /// An exported sheet must load back as the same animations — that round
    /// trip is the whole point of the export.
    #[test]
    fn exported_sheet_reloads_identically() {
        let kind = Kind::Mouse;
        let set = builtin(kind, &kind.palette());

        let dir = std::env::temp_dir().join("petpal-export-test");
        let _ = std::fs::remove_dir_all(&dir);
        export_sheet(&set, &dir).expect("export");

        let back = crate::sprites::load_sheet(&dir).expect("reload");
        assert_eq!(back.w, set.w);
        assert_eq!(back.h, set.h);
        // The loader slices the whole 8-wide grid, so a sheet whose frame count
        // is not a multiple of 8 comes back with a few blank trailing cells.
        // They are unreferenced by any clip; what has to match is the clips.
        assert!(back.frames.len() >= set.frames.len());
        for a in Anim::ALL {
            assert_eq!(back.clip(a).idx, set.clip(a).idx, "{:?} frames", a);
            assert_eq!(back.clip(a).frame_ms, set.clip(a).frame_ms, "{:?} ms", a);
        }
        // Spot-check that the pixels survived the premultiply round trip.
        let (a, b) = (set.frame(Anim::Idle, 0), back.frame(Anim::Idle, 0));
        assert_eq!(a.px, b.px);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn icon_has_every_size() {
        let set = builtin(Kind::Pal, &Kind::Pal.palette());
        let ico = build_icon(&set).unwrap();
        assert_eq!(u16::from_le_bytes([ico[2], ico[3]]), 1, "type must be icon");
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        assert_eq!(count, 7);
        // Every entry must point inside the file.
        for i in 0..count {
            let e = 6 + i * 16;
            let len = u32::from_le_bytes(ico[e + 8..e + 12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(ico[e + 12..e + 16].try_into().unwrap()) as usize;
            assert!(len > 0 && off + len <= ico.len(), "entry {i} out of bounds");
        }
    }
}
