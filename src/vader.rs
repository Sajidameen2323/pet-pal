//! The second built-in creature: a chibi Sith lord.
//!
//! Shares the pose vocabulary and frame timings in [`crate::sprites`] — only
//! the drawing differs. Where Pal is built from ellipses, this one is built
//! from boxes, which is what makes it read as armour at 32x32.
//!
//! The hard part of drawing a black character in pixel art is that it
//! disappears against a dark background, so the silhouette is drawn as a *rim
//! light* (`pal.outline` is a pale blue-grey here, not a dark outline) and the
//! interior detail comes from the lighter grey panels in `pal.belly`.

use crate::sprites::{draw_z, fade, lift_of, Eyes, Fx, Legs, Palette, Pose, Raster, CW, FEET, SWING};

/// Reuse the pose fields with Sith semantics: `tail` sways the cape and
/// `ears_back` billows it. The sabre ignites on the annoyed animation — a
/// frustrated Sith reaching for his blade is a better reading of "the CPU is
/// pegged" than Pal's puffs of steam.
pub(crate) fn draw(r: &mut Raster, p: &Pose, pal: &Palette) {
    if p.lying {
        draw_lying(r, p, pal);
    } else {
        draw_standing(r, p, pal);
    }
    draw_fx(r, p, pal);
}

fn draw_standing(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = CW / 2 + p.lean;
    let dy = p.body_dy;

    // Cape first — everything else stacks on top of it. It stops above the
    // ankles: hanging it to the floor turns the whole figure into a black
    // pyramid and hides the walk cycle completely.
    draw_cape(r, ox, 14 + dy, FEET - 4, p.tail, p.ears_back, pal);
    draw_legs(r, ox, dy, p.legs, pal);

    // Torso, then the wider shoulder mantle over its top.
    r.boxed(ox - 5, 15 + dy, ox + 5, 24 + dy, pal.body, pal.outline);
    r.boxed(ox - 7, 13 + dy, ox + 7, 16 + dy, pal.body, pal.outline);

    draw_chest_panel(r, ox, 18 + dy, pal);

    // Belt, with the buckle boxes that read as the utility belt.
    r.boxed(ox - 6, 22 + dy, ox + 6, 24 + dy, pal.belly, pal.outline);
    r.put(ox - 3, 23 + dy, pal.body);
    r.put(ox, 23 + dy, pal.body);
    r.put(ox + 3, 23 + dy, pal.body);

    draw_helmet(r, ox + p.head_dx, 8 + p.head_dy + dy / 2, p, pal);

    if let Fx::Steam(ph) = p.fx {
        draw_sabre(r, ox + 8, 19 + dy, ph, pal);
    }
}

/// Asleep: the cape becomes a bedroll and the helmet rests on the left, mirroring
/// how Pal curls up so the two creatures read the same way at a glance.
fn draw_lying(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = CW / 2 + 2;
    let by = FEET - 3 + p.body_dy;
    let bry = 4 + p.squash;

    // Cape pooled on the ground.
    r.boxed(ox - 10, by - bry, ox + 10, by + bry, pal.body, pal.outline);
    r.rect(ox - 2, by - 1, ox + 7, by + 1, pal.belly);

    // Boots poking out of the far end.
    r.boxed(ox + 6, by + bry - 2, ox + 10, by + bry, pal.body, pal.outline);

    let hx = ox - 9 + p.head_dx;
    let hy = by - 4 + p.head_dy;
    draw_helmet(r, hx, hy, p, pal);
}

fn draw_helmet(r: &mut Raster, hx: i32, hy: i32, p: &Pose, pal: &Palette) {
    // Domed crown.
    r.blob(hx, hy, 5, 5, pal.body, pal.outline);
    // Flared cheek panels — the widest part of the silhouette.
    r.boxed(hx - 7, hy + 1, hx + 7, hy + 6, pal.body, pal.outline);
    // Face plate sits proud of the flares.
    r.boxed(hx - 5, hy - 2, hx + 5, hy + 6, pal.body, pal.outline);

    // Brow ridge, dropped a row when angry so the whole mask scowls.
    let brow = if p.eyes == Eyes::Angry { 1 } else { 0 };
    r.hline(hx - 5, hx + 5, hy - 1 + brow, pal.outline);

    draw_lenses(r, hx, hy + 1 + brow, p.eyes, pal);
    // Nose ridge between the lenses. Without it the two of them merge into a
    // single red bar at this size and the mask loses its face.
    r.rect(hx - 1, hy + brow, hx + 1, hy + 3, pal.body);
    r.put(hx, hy + 1 + brow, pal.outline);

    // Mouth grille: a lighter plate with vertical vents cut into it.
    r.rect(hx - 3, hy + 4, hx + 3, hy + 5, pal.belly);
    for dx in [-2, 0, 2] {
        r.put(hx + dx, hy + 4, pal.body);
        r.put(hx + dx, hy + 5, pal.body);
    }
    // Chin vent.
    r.rect(hx - 2, hy + 7, hx + 2, hy + 7, pal.belly);
}

fn draw_lenses(r: &mut Raster, cx: i32, cy: i32, e: Eyes, pal: &Palette) {
    // The lenses carry the whole expression: the mask cannot emote, so state is
    // read entirely from how hot they glow and how they are angled.
    let glow = match e {
        Eyes::Closed => fade(pal.eye, 0.35),
        Eyes::Angry => pal.blush,
        Eyes::Wide | Eyes::Happy => pal.eye,
        _ => fade(pal.eye, 0.8),
    };

    for &dx in &[-4i32, 4] {
        let x = cx + dx;
        // Inward direction, so both lenses slant toward the nose.
        let s = if dx < 0 { 1 } else { -1 };
        match e {
            Eyes::Closed => {
                // Powered down: a single dim pixel.
                r.rect(x - 1, cy, x + 1, cy, pal.body);
                r.put(x, cy, glow);
            }
            Eyes::Angry => {
                // Narrowed and tilted.
                r.rect(x - 1, cy, x + 1, cy + 1, pal.body);
                r.put(x - s, cy, glow);
                r.put(x, cy, glow);
                r.put(x + s, cy + 1, glow);
            }
            Eyes::Wide => {
                r.rect(x - 1, cy - 1, x + 1, cy + 1, pal.body);
                r.rect(x - 1, cy, x + 1, cy, glow);
                r.put(x, cy - 1, glow);
                r.put(x, cy + 1, glow);
            }
            _ => {
                r.rect(x - 1, cy, x + 1, cy + 1, pal.body);
                r.rect(x - 1, cy, x + 1, cy, glow);
            }
        }
    }
}

fn draw_chest_panel(r: &mut Raster, ox: i32, y: i32, pal: &Palette) {
    r.boxed(ox - 4, y, ox + 4, y + 4, pal.belly, pal.outline);
    // The three indicator lights everyone remembers.
    r.put(ox - 2, y + 1, crate::sprites::argb(255, 0xE0, 0x36, 0x36));
    r.put(ox, y + 1, crate::sprites::argb(255, 0x3F, 0xC0, 0x62));
    r.put(ox + 2, y + 1, crate::sprites::argb(255, 0x52, 0x95, 0xE8));
    // Readout rows below them.
    r.hline(ox - 2, ox + 2, y + 3, pal.body);
}

/// A trapezoid widening toward the floor, offset by `sway` so it trails the
/// walk cycle. Both edges get the rim light, which is most of what sells the
/// shape against a dark background.
fn draw_cape(r: &mut Raster, ox: i32, top: i32, bottom: i32, sway: i32, billow: bool, pal: &Palette) {
    let span = (bottom - top).max(1) as f32;
    let flare = if billow { 7.0 } else { 4.5 };
    for y in top..=bottom {
        let t = (y - top) as f32 / span;
        let half = (5.0 + t * flare) as i32;
        let shift = (sway as f32 * t * 1.6) as i32;
        let (l, rr) = (ox - half + shift, ox + half + shift);
        r.hline(l, rr, y, pal.body);
        r.put(l, y, pal.outline);
        r.put(rr, y, pal.outline);
    }
    // Hem.
    let t_end = 1.0;
    let half = (5.0 + t_end * flare) as i32;
    let shift = (sway as f32 * t_end * 1.6) as i32;
    r.hline(ox - half + shift, ox + half + shift, bottom, pal.outline);
}

fn draw_legs(r: &mut Raster, ox: i32, dy: i32, legs: Legs, pal: &Palette) {
    let boot = |r: &mut Raster, x: i32, y: i32| {
        r.boxed(x - 3, y - 6, x + 2, y, pal.body, pal.outline);
        // Shin plate catches the light.
        r.rect(x - 2, y - 5, x + 1, y - 3, pal.belly);
    };
    match legs {
        Legs::Stand => {
            boot(r, ox - 3, FEET + dy.min(0));
            boot(r, ox + 4, FEET + dy.min(0));
        }
        Legs::Walk(ph) => {
            let swing = SWING[(ph % 8) as usize];
            boot(r, ox - 3 + swing, FEET - lift_of(swing));
            boot(r, ox + 4 - swing, FEET - lift_of(-swing));
        }
        Legs::Tuck => {
            r.boxed(ox - 7, FEET - 3, ox + 7, FEET, pal.body, pal.outline);
        }
        Legs::Splay => {
            boot(r, ox - 7, FEET - 3);
            boot(r, ox + 8, FEET - 3);
        }
        Legs::Dangle => {
            boot(r, ox - 3, FEET);
            boot(r, ox + 4, FEET);
        }
        Legs::Cling(ph) => {
            // There is no room for a raised boot clear of the cape at this
            // size, and one hanging in space beside him reads as cargo rather
            // than a leg. So the climb comes from a stagger: the lead boot up
            // on the edge at hip height, just past the torso, and the trailing
            // one driving from the bottom of the frame.
            let s = SWING[(ph % 8) as usize];
            boot(r, ox + 8, FEET - 4 - s);
            boot(r, ox + 1, FEET + s / 2);
        }
    }
}

/// Ignited sabre: a dark hilt, a hot white core and a red bloom around it.
/// `ph` flickers the blade length so it hums rather than sitting static.
fn draw_sabre(r: &mut Raster, hx: i32, hy: i32, ph: u8, pal: &Palette) {
    // Hilt in the fist.
    r.boxed(hx - 1, hy - 1, hx + 1, hy + 2, pal.belly, pal.outline);

    let flicker = [0i32, 1, 0, -1][(ph % 4) as usize];
    let (tip_x, tip_y) = (hx + 5, hy - 15 - flicker);
    let steps = 18;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = hx as f32 + (tip_x - hx) as f32 * t;
        let y = (hy - 2) as f32 + (tip_y - (hy - 2)) as f32 * t;
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        // Red bloom either side of a white-hot core.
        r.put(xi - 1, yi, fade(pal.blush, 0.5));
        r.put(xi + 1, yi, fade(pal.blush, 0.5));
        r.put(xi, yi, if t > 0.06 { pal.accent } else { pal.blush });
    }
}

fn draw_fx(r: &mut Raster, p: &Pose, pal: &Palette) {
    match p.fx {
        Fx::None => {}
        Fx::Zzz(ph) => {
            let base_x = if p.lying { 13 } else { 19 };
            for i in 0..3u8 {
                let t = ((ph + i * 2) % 6) as f32 / 6.0;
                let x = base_x + i as i32 * 4 + (t * 2.0) as i32;
                let y = 16 - i as i32 * 5 - (t * 3.0) as i32;
                let size = 3 - (i as i32).min(1);
                draw_z(r, x, y, size, fade(pal.accent, 1.0 - t * 0.7), pal.outline);
            }
        }
        // The annoyed animation's effect is the sabre, drawn with the body so
        // the arm holds it rather than it floating alongside.
        Fx::Steam(_) => {}
        Fx::Bang(ph) => {
            let bob = [0i32, -1, -2, -1][(ph % 4) as usize];
            let x = 26;
            let y = 3 + bob;
            r.rect(x - 2, y - 1, x + 2, y + 5, pal.outline);
            r.rect(x - 1, y, x + 1, y + 4, pal.accent);
            r.rect(x - 2, y + 6, x + 2, y + 8, pal.outline);
            r.put(x, y + 7, pal.accent);
        }
        Fx::Spark(ph) => {
            // Same flourish as the other creatures, in his own colours.
            const AT: [(i32, i32); 3] = [(11, 8), (21, 4), (28, 10)];
            for (i, &(bx, by)) in AT.iter().enumerate() {
                let t = ((ph as usize + i * 2) % 6) as f32 / 6.0;
                let size = if t < 0.2 || t > 0.8 { 0 } else if t < 0.5 { 2 } else { 1 };
                if size == 0 {
                    continue;
                }
                let y = by - (t * 3.0) as i32;
                let c = fade(pal.accent, 1.0 - t * 0.5);
                r.hline(bx - size, bx + size, y, c);
                r.rect(bx, y - size, bx, y + size, c);
            }
        }
    }
}
