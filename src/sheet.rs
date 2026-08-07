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
    use crate::sprites::{builtin, Kind};

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
