//! The third built-in creature: a small grey quadruped mouse.
//!
//! Pal and Vader are bipeds drawn front-on; this one is a side-view animal, so
//! the shared [`Pose`] fields get read differently again: `legs` drives a
//! four-beat gait rather than two feet, `ears_back` flattens the ears against
//! the skull, and `lean` stretches the body forward into a run.
//!
//! It carries more detail than the other two — far-side limbs in a darker
//! shade, an inner ear, whiskers, a tapering tail — because a mouse silhouette
//! is mostly one grey lozenge and needs the interior lines to read as an animal
//! rather than a pebble.

use crate::sprites::{draw_fx, fade, lift_of, Eyes, Legs, Palette, Pose, Raster, CW, FEET, SWING};

/// Blend `a` toward `b`. Used for the far-side limbs, which sit in shadow.
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let x = ((a >> sh) & 0xff) as f32;
        let y = ((b >> sh) & 0xff) as f32;
        ((x + (y - x) * t.clamp(0.0, 1.0)) as u32).min(255)
    };
    (a & 0xff00_0000) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

pub(crate) fn draw(r: &mut Raster, p: &Pose, pal: &Palette) {
    if p.lying {
        draw_curled(r, p, pal);
    } else {
        draw_standing(r, p, pal);
    }
    draw_fx(r, p, pal);
}

fn draw_standing(r: &mut Raster, p: &Pose, pal: &Palette) {
    // A running mouse stretches out; `lean` pushes the head forward and the
    // haunches back rather than tilting the whole body like a biped.
    //
    // Climbing wants the opposite of a stretch. On an upright creature `lean`
    // means "pressed into the wall"; here that has to become a bunch, head
    // drawn back over the shoulders, or the mouse scales a window looking like
    // it is sprinting flat out.
    let stretch = match p.legs {
        Legs::Cling(_) => -p.lean / 2,
        _ => p.lean,
    };
    let ox = CW / 2;
    let by = 24 + p.body_dy;
    let bry = 4 + p.squash;

    // Far-side limbs sit in shadow so the near ones read as nearer.
    let far = blend(pal.body, pal.outline, 0.45);

    draw_legs(r, ox, stretch, p.legs, far, pal);

    // Haunch first, so the body outline runs over its front edge.
    r.blob(ox - 4 - stretch, by - 1, 5, 4, pal.body, pal.outline);
    // Barrel of the body.
    r.blob(ox, by, 8, bry, pal.body, pal.outline);
    // Pale underside.
    r.ellipse(ox + 1, by + bry - 2, 6, 1, pal.belly);

    // Tail over the hip rather than behind it. Drawn underneath, its root is
    // swallowed by the body and the rest reads as a stray arc floating in space.
    draw_tail(r, ox - 6 - stretch, by + 1, p.tail, pal);

    draw_head(r, ox + 8 + stretch, by - 2 + p.head_dy, p, far, pal);
}

/// Asleep: curled nose to tail with the tail wrapped around the outside.
fn draw_curled(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = CW / 2;
    let by = FEET - 5 + p.body_dy;
    let rad = 6 + p.squash;

    // Tail curls right around the sleeping ball.
    let ring = [
        (ox - 9, by + 3),
        (ox - 9, by),
        (ox - 8, by - 4),
        (ox - 5, by - 7),
        (ox - 1, by - 8),
    ];
    for (i, &(x, y)) in ring.iter().enumerate() {
        let t = i as f32 / ring.len() as f32;
        r.blob(x, y, 1, 1, blend(pal.blush, pal.body, t), pal.outline);
    }

    r.blob(ox, by, rad + 2, rad, pal.body, pal.outline);
    r.ellipse(ox, by + rad - 2, 5, 1, pal.belly);

    // Head tucked into the flank, ears folded down.
    let hx = ox + 5;
    let hy = by + 1;
    r.blob(hx - 3, hy - 5, 3, 2, blend(pal.body, pal.outline, 0.2), pal.outline);
    r.blob(hx, hy, 4, 3, pal.body, pal.outline);
    // Closed eye and a snout resting on the paws.
    r.hline(hx, hx + 2, hy - 1, pal.eye);
    r.blob(hx + 4, hy + 1, 2, 1, pal.belly, pal.outline);
    r.put(hx + 6, hy + 1, pal.blush);
}

fn draw_head(r: &mut Raster, hx: i32, hy: i32, p: &Pose, far: u32, pal: &Palette) {
    // Far ear peeks over the skull, in shadow, to give the head depth.
    let (ear_dx, ear_dy) = if p.ears_back { (-3, -2) } else { (-1, -5) };
    r.blob(hx + ear_dx - 3, hy + ear_dy, 2, 2, far, pal.outline);

    // Skull, then the wedge of the snout.
    r.blob(hx, hy, 4, 4, pal.body, pal.outline);
    r.blob(hx + 4, hy + 2, 3, 2, pal.body, pal.outline);
    r.ellipse(hx + 4, hy + 3, 2, 1, pal.belly);

    // Near ear: big and round, with a pink inner cup.
    let (nx, ny) = (hx + ear_dx, hy + ear_dy);
    r.blob(nx, ny, 3, 3, pal.body, pal.outline);
    r.ellipse(nx + 1, ny, 1, 1, pal.blush);

    draw_eye(r, hx + 2, hy, p.eyes, pal);

    // Nose and whiskers.
    let (tipx, tipy) = (hx + 7, hy + 3);
    r.put(tipx, tipy, pal.blush);
    let lift = if p.eyes == Eyes::Angry { 1 } else { 0 };
    for (dy, len) in [(-2 - lift, 4), (0, 4), (2, 3)] {
        let w = fade(pal.accent, 0.5);
        for i in 1..=len {
            r.put(tipx + i, tipy + dy * i / len.max(1), w);
        }
    }
}

fn draw_eye(r: &mut Raster, x: i32, y: i32, e: Eyes, pal: &Palette) {
    match e {
        Eyes::Closed => {
            r.hline(x - 1, x + 1, y, pal.eye);
        }
        Eyes::Half => {
            r.hline(x - 1, x + 1, y, pal.eye);
            r.put(x, y + 1, pal.eye);
        }
        Eyes::Angry => {
            r.put(x, y, pal.eye);
            r.put(x + 1, y, pal.eye);
            // Brow dropped toward the snout.
            r.put(x + 1, y - 2, pal.eye);
            r.put(x + 2, y - 1, pal.eye);
        }
        Eyes::Wide => {
            r.blob(x, y, 1, 1, pal.eye, pal.eye);
            r.put(x - 1, y - 1, pal.accent);
        }
        Eyes::Happy => {
            r.put(x - 1, y + 1, pal.eye);
            r.put(x, y, pal.eye);
            r.put(x + 1, y + 1, pal.eye);
        }
        Eyes::Open => {
            r.ellipse(x, y, 1, 1, pal.eye);
            r.put(x, y - 1, pal.accent);
        }
    }
}

/// Long and thin, curling up behind and sweeping with `sway`.
///
/// Walked pixel by pixel along the control points rather than plotting them
/// directly: at three pixels apart the raw points render as a line of
/// disconnected dots instead of a tail.
fn draw_tail(r: &mut Raster, bx: i32, by: i32, sway: i32, pal: &Palette) {
    let pts = [
        (bx, by),
        (bx - 3, by),
        (bx - 5, by - 2 + sway),
        (bx - 6, by - 5 + sway),
        (bx - 5, by - 8 + sway * 2),
        (bx - 3, by - 10 + sway * 2),
    ];

    // Shadow pass first, offset down-right, then the tail itself over it.
    for (dx, dy, shadow) in [(1, 1, true), (0, 0, false)] {
        let mut travelled = 0.0f32;
        for w in pts.windows(2) {
            let (x0, y0) = w[0];
            let (x1, y1) = w[1];
            let len = (((x1 - x0).pow(2) + (y1 - y0).pow(2)) as f32).sqrt().max(1.0);
            let steps = len.ceil() as i32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let x = x0 as f32 + (x1 - x0) as f32 * t;
                let y = y0 as f32 + (y1 - y0) as f32 * t;
                let c = if shadow {
                    pal.outline
                } else {
                    // Flesh-pink at the base, fading toward the body colour.
                    blend(pal.blush, pal.body, (travelled + t * len) / 22.0)
                };
                r.put(x.round() as i32 + dx, y.round() as i32 + dy, c);
            }
            travelled += len;
        }
    }
}

/// Four legs: the far pair in shadow, swinging in antiphase with the near pair.
fn draw_legs(r: &mut Raster, ox: i32, stretch: i32, legs: Legs, far: u32, pal: &Palette) {
    let py = FEET;
    let (front, back) = (ox + 6 + stretch, ox - 5 - stretch);

    let near = blend(pal.body, pal.belly, 0.45);
    let paw = |r: &mut Raster, x: i32, y: i32, c: u32| {
        r.blob(x, y, 1, 1, c, pal.outline);
    };

    match legs {
        Legs::Stand => {
            paw(r, front - 2, py, far);
            paw(r, back + 2, py, far);
            paw(r, front, py, near);
            paw(r, back, py, near);
        }
        Legs::Walk(ph) => {
            // Diagonal pairs, as a real quadruped moves.
            let a = SWING[(ph % 8) as usize];
            let b = -a;
            paw(r, front - 2 + b, py - lift_of(b), far);
            paw(r, back + 2 + a, py - lift_of(a), far);
            paw(r, front + a, py - lift_of(a), near);
            paw(r, back + b, py - lift_of(b), near);
        }
        Legs::Tuck => {
            // Sitting up on the haunches, forepaws held at the chest.
            paw(r, back, py, near);
            paw(r, back + 4, py, far);
            paw(r, front - 1, py - 3, near);
        }
        Legs::Splay => {
            paw(r, front + 2, py - 3, near);
            paw(r, back - 2, py - 3, near);
            paw(r, front, py - 1, far);
            paw(r, back, py - 1, far);
        }
        Legs::Dangle => {
            paw(r, front - 1, py, near);
            paw(r, back + 1, py, near);
            paw(r, front - 3, py - 2, far);
            paw(r, back + 3, py - 2, far);
        }
        Legs::Cling(ph) => {
            // A reaching forelimb has nowhere to go on this creature: the snout
            // already runs to the right edge of the cell, and the barrel covers
            // everything above row 28. So the climb reads from all four legs
            // bunched forward under the chest, scrabbling in antiphase, instead
            // of from a big overhead grab.
            let a = SWING[(ph % 8) as usize];
            let b = -a;
            paw(r, front - 1, py - lift_of(a), near);
            paw(r, front - 3, py + 1 - lift_of(b), far);
            paw(r, front - 5, py - lift_of(b), near);
            paw(r, front - 7, py + 1 - lift_of(a), far);
        }
    }
}
