//! The fourth built-in creature: a small brown monkey.
//!
//! The other three all fight the same problem — limbs are drawn *before* the
//! body, so anything a pose lifts into the torso's silhouette is painted over
//! and disappears. That is why Vader cannot raise a boot and the mouse cannot
//! reach overhead.
//!
//! This one is layered properly: far limbs, then the body, then the near limbs
//! on top. A monkey is defined by its arms — hanging past its knees, reaching
//! overhead, gripping a ledge — and none of that reads if the arm vanishes the
//! moment it crosses the chest. It costs one extra pass over the frame.
//!
//! Pose fields are read as: `lean` tilts the torso forward and carries the head
//! with it, `tail` sways the tail, `ears_back` flattens the ears against the
//! skull, and `legs` drives both the feet and the arms — they swing in
//! antiphase, which is what makes a walk look like a walk.

use crate::sprites::{
    draw_fx_at, fade, lift_of, Eyes, Legs, Palette, Pose, Raster, FEET, SWING,
};

/// Frame width. Wider than the square creatures: a monkey with its arms down is
/// already as wide as it is tall, and the tail needs somewhere to curl.
pub(crate) const FW: i32 = 36;

/// Blend `a` toward `b`. Far-side limbs sit in shadow so the near ones read as
/// nearer — the same trick the mouse uses.
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let x = ((a >> sh) & 0xff) as f32;
        let y = ((b >> sh) & 0xff) as f32;
        ((x + (y - x) * t.clamp(0.0, 1.0)) as u32).min(255)
    };
    (a & 0xff00_0000) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

pub(crate) fn draw(r: &mut Raster, p: &Pose, pal: &Palette) {
    // Effects anchor on the head, and this creature's head is nowhere near the
    // middle of its cell — the hips are. Laid out for the centre, the steam
    // cloud sits on its face. The extra 2px past the skull centre clears the
    // ear, which sticks out further than any other creature's.
    let head_x = if p.lying {
        draw_curled(r, p, pal);
        r.w / 2 + 8
    } else {
        draw_standing(r, p, pal);
        r.w / 2 + 5 + p.lean * 2 + p.head_dx
    };
    draw_fx_at(r, p, pal, head_x);
}

/// Where a limb ends, in frame coordinates.
struct Grip {
    hand: (i32, i32),
    foot: (i32, i32),
}

fn draw_standing(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = r.w / 2;
    let tilt = p.lean;
    // Hips sit low; the chest and head stack up from there.
    let hy = 22 + p.body_dy;
    let cy = hy - 6 + p.squash;

    let far = blend(pal.body, pal.outline, 0.42);
    let dark = blend(pal.body, pal.outline, 0.2);

    // Shoulder and hip sockets — every limb is measured from these, so a tilt
    // or a bob moves the whole rig together instead of shearing it.
    let shoulder = (ox + tilt, cy);
    let hip = (ox - 1, hy + 2);

    draw_tail(r, ox - 5, hy + 1, p.tail, pal);

    // --- far side, under the body ---
    let g = grip(p.legs, ox, hy, true);
    limb(r, shoulder, g.hand, 2, far, far);
    limb(r, hip, g.foot, 2, far, far);
    foot(r, g.foot, far, far);

    // --- body ---
    // Haunches, then a narrower chest, so the silhouette is a pear rather than
    // an egg. Apes read as bottom-heavy.
    r.blob(ox - 1, hy, 6, 5, pal.body, pal.outline);
    r.blob(ox + tilt, cy, 5, 5, pal.body, pal.outline);
    // Pale chest and belly, in one continuous patch down the front.
    r.ellipse(ox + 1 + tilt, cy + 1, 3, 4, pal.belly);
    r.ellipse(ox + 1, hy, 3, 3, pal.belly);

    draw_head(
        r,
        ox + 3 + tilt * 2 + p.head_dx,
        cy - 7 + p.head_dy,
        p,
        far,
        dark,
        pal,
    );

    // --- near side, over the body ---
    // This is the whole point of the layering: a hand raised to the creature's
    // own chin still shows.
    let g = grip(p.legs, ox, hy, false);
    limb(r, shoulder, g.hand, 2, pal.body, pal.outline);
    hand(r, g.hand, pal);
    limb(r, hip, g.foot, 2, pal.body, pal.outline);
    foot(r, g.foot, pal.belly, pal.outline);
}

/// Asleep: curled on its side with the tail wrapped round and the arms tucked
/// under the chin.
fn draw_curled(r: &mut Raster, p: &Pose, pal: &Palette) {
    let ox = r.w / 2;
    let by = FEET - 5 + p.body_dy;
    let rad = 6 + p.squash;

    // Tail loops all the way around the sleeping ball.
    let ring = [
        (ox - 10, by + 3),
        (ox - 11, by - 1),
        (ox - 9, by - 5),
        (ox - 5, by - 8),
        (ox, by - 9),
    ];
    for (i, &(x, y)) in ring.iter().enumerate() {
        let t = i as f32 / ring.len() as f32;
        r.blob(x, y, 1, 1, blend(pal.body, pal.blush, t), pal.outline);
    }

    r.blob(ox, by, rad + 2, rad, pal.body, pal.outline);
    r.ellipse(ox + 1, by + rad - 2, 5, 1, pal.belly);

    // Head resting on the flank, ears folded flat.
    let (hx, hy) = (ox + 6, by - 1);
    r.blob(hx - 4, hy - 4, 2, 2, blend(pal.body, pal.outline, 0.25), pal.outline);
    r.blob(hx, hy, 5, 4, pal.body, pal.outline);
    r.ellipse(hx + 1, hy + 1, 3, 2, pal.belly);
    // Closed eyes and a slack mouth.
    r.hline(hx, hx + 2, hy - 1, pal.eye);
    r.put(hx + 2, hy + 2, pal.eye);
    // A hand tucked under the chin.
    r.blob(hx - 1, hy + 3, 2, 1, pal.belly, pal.outline);
}

// ---------------------------------------------------------------------------
// Limb rig
// ---------------------------------------------------------------------------

/// Where this pose puts the hand and foot on one side of the body.
///
/// `far` is the side away from the viewer; it trails the near side by half a
/// cycle, which is what stops a four-limbed walk looking like a hop.
fn grip(legs: Legs, ox: i32, hy: i32, far: bool) -> Grip {
    let ground = FEET;
    // Arms hang past the knee — the single most monkey-ish thing about the
    // silhouette, so the resting hand sits low rather than at the hip.
    let rest_hand = ground - 5;
    let side = if far { -2 } else { 2 };

    match legs {
        Legs::Stand => Grip {
            // Clear of the flank, so the arm reads as an arm and not as shading
            // on the chest.
            hand: (ox + 7 + side / 2, rest_hand),
            foot: (ox - 1 + side, ground),
        },
        Legs::Walk(ph) => {
            // Arms counter-swing against the legs, as they do on a real gait.
            let ph = if far { ph + 4 } else { ph };
            let s = SWING[(ph % 8) as usize];
            Grip {
                hand: (ox + 6 - s, rest_hand - lift_of(s) / 2),
                foot: (ox - 1 + s, ground - lift_of(s)),
            }
        }
        Legs::Tuck => Grip {
            // Sitting: knees drawn up in front, hands folded on top of them.
            hand: (ox + 3 + side / 2, ground - 6),
            foot: (ox + 5 + side, ground),
        },
        Legs::Splay => Grip {
            // Airborne, everything thrown out.
            hand: (ox + 9 + side, hy - 8),
            foot: (ox - 7 - side, ground - 3),
        },
        Legs::Dangle => Grip {
            // Held up by the scruff: arms and legs hang slack and straight
            // down, past the bottom of the body.
            hand: (ox + 4 + side / 2, ground + 1),
            foot: (ox - 2 + side, ground + 2),
        },
        Legs::Cling(ph) => {
            // Hand over hand up the edge, the free foot pushing off below.
            // Drawn over the body, so the raised arm genuinely reaches.
            let ph = if far { ph + 4 } else { ph };
            let s = SWING[(ph % 8) as usize];
            Grip {
                hand: (ox + 7, hy - 12 - s),
                foot: (ox + 4, ground - 4 + s / 2),
            }
        }
    }
}

/// A limb as a tapering line from socket to extremity, outlined then filled so
/// it keeps the chunky look of the blobs it grows out of.
fn limb(r: &mut Raster, from: (i32, i32), to: (i32, i32), w: i32, fill: u32, line: u32) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let len = (((x1 - x0).pow(2) + (y1 - y0).pow(2)) as f32).sqrt().max(1.0);
    let steps = len.ceil() as i32;
    for pass in 0..2 {
        let (c, extra) = if pass == 0 { (line, 1) } else { (fill, 0) };
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = (x0 as f32 + (x1 - x0) as f32 * t).round() as i32;
            let y = (y0 as f32 + (y1 - y0) as f32 * t).round() as i32;
            // Thins slightly toward the hand or foot.
            let hw = w - (t * 1.0) as i32 + extra;
            r.rect(x - hw / 2, y - hw / 2, x + hw - hw / 2 - 1, y + hw - hw / 2 - 1, c);
        }
    }
}

fn hand(r: &mut Raster, at: (i32, i32), pal: &Palette) {
    r.blob(at.0, at.1, 1, 1, pal.belly, pal.outline);
}

fn foot(r: &mut Raster, at: (i32, i32), fill: u32, line: u32) {
    r.blob(at.0, at.1, 2, 0, fill, line);
}

// ---------------------------------------------------------------------------
// Head
// ---------------------------------------------------------------------------

fn draw_head(
    r: &mut Raster,
    hx: i32,
    hy: i32,
    p: &Pose,
    far: u32,
    dark: u32,
    pal: &Palette,
) {
    // Ears: big round discs on the sides of the skull. The far one is in
    // shadow and drawn first so the head overlaps it.
    let (ear_dx, ear_dy) = if p.ears_back { (4, 1) } else { (6, -1) };
    r.blob(hx - ear_dx, hy + ear_dy, 2, 2, far, pal.outline);
    r.blob(hx - ear_dx + 1, hy + ear_dy, 1, 1, dark, dark);

    r.blob(hx, hy, 5, 5, pal.body, pal.outline);

    // Near ear over the skull.
    r.blob(hx + ear_dx, hy + ear_dy, 2, 2, pal.body, pal.outline);
    r.ellipse(hx + ear_dx, hy + ear_dy, 1, 1, pal.blush);

    // The pale face mask is what makes this read as a monkey rather than a
    // bear: a rounded heart shape covering brow, eyes and muzzle.
    r.ellipse(hx + 1, hy, 3, 4, pal.belly);
    r.ellipse(hx + 1, hy + 3, 3, 2, pal.belly);

    draw_eyes(r, hx, hy - 1, p.eyes, pal);

    // Muzzle: a slightly darker snout with a nostril pair and a mouth line.
    r.ellipse(hx + 2, hy + 3, 2, 1, blend(pal.belly, pal.blush, 0.55));
    r.put(hx + 1, hy + 3, pal.eye);
    r.put(hx + 3, hy + 3, pal.eye);
    draw_mouth(r, hx + 2, hy + 4, p.eyes, pal);

    if p.blush {
        r.put(hx - 2, hy + 2, fade(pal.blush, 0.75));
        r.put(hx + 5, hy + 2, fade(pal.blush, 0.75));
    }
}

fn draw_eyes(r: &mut Raster, hx: i32, hy: i32, e: Eyes, pal: &Palette) {
    // Set close together and forward-facing, which is what gives primates their
    // expression. Two pixels apart is the whole difference between a monkey and
    // a rodent at this size.
    for (i, x) in [hx, hx + 3].into_iter().enumerate() {
        let outer = i == 1;
        match e {
            Eyes::Closed => r.hline(x - 1, x, hy, pal.eye),
            Eyes::Half => {
                r.hline(x - 1, x, hy, pal.eye);
                r.put(x, hy + 1, pal.eye);
            }
            Eyes::Angry => {
                r.put(x, hy, pal.eye);
                // Brow dropped toward the middle of the face.
                let bx = if outer { x - 1 } else { x + 1 };
                r.put(bx, hy - 2, pal.eye);
                r.put(x, hy - 1, pal.eye);
            }
            Eyes::Wide => {
                r.blob(x, hy, 1, 1, pal.accent, pal.eye);
                r.put(x, hy, pal.eye);
            }
            Eyes::Happy => {
                r.put(x - 1, hy + 1, pal.eye);
                r.put(x, hy, pal.eye);
                r.put(x + 1, hy + 1, pal.eye);
            }
            Eyes::Open => {
                r.put(x, hy, pal.eye);
                r.put(x, hy + 1, pal.eye);
                r.put(x, hy - 1, pal.accent);
            }
        }
    }
}

fn draw_mouth(r: &mut Raster, cx: i32, cy: i32, e: Eyes, pal: &Palette) {
    match e {
        Eyes::Angry => {
            r.put(cx - 2, cy + 1, pal.eye);
            r.hline(cx - 1, cx + 1, cy, pal.eye);
            r.put(cx + 2, cy + 1, pal.eye);
        }
        Eyes::Closed => r.hline(cx - 1, cx, cy, pal.eye),
        // The open-mouthed hoot, for anything excited.
        Eyes::Happy | Eyes::Wide => {
            r.blob(cx, cy, 1, 1, blend(pal.blush, pal.eye, 0.5), pal.eye);
        }
        _ => r.hline(cx - 1, cx + 1, cy, pal.eye),
    }
}

// ---------------------------------------------------------------------------
// Tail
// ---------------------------------------------------------------------------

/// A long prehensile tail, curling up behind and sweeping with `sway`.
///
/// Walked pixel by pixel along the control points: at four pixels apart the raw
/// points render as a line of dots rather than a tail.
fn draw_tail(r: &mut Raster, bx: i32, by: i32, sway: i32, pal: &Palette) {
    let pts = [
        (bx, by),
        (bx - 4, by + 2),
        (bx - 8, by + sway),
        (bx - 10, by - 5 + sway),
        (bx - 8, by - 10 + sway * 2),
        (bx - 4, by - 12 + sway * 2),
    ];

    // Outline pass offset down-left, then the tail over it, so it keeps an edge
    // against the window behind it.
    for (dx, dy, shadow) in [(-1, 1, true), (0, 0, false)] {
        for w in pts.windows(2) {
            let (x0, y0) = w[0];
            let (x1, y1) = w[1];
            let len = (((x1 - x0).pow(2) + (y1 - y0).pow(2)) as f32).sqrt().max(1.0);
            let steps = len.ceil() as i32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let x = (x0 as f32 + (x1 - x0) as f32 * t).round() as i32;
                let y = (y0 as f32 + (y1 - y0) as f32 * t).round() as i32;
                let c = if shadow { pal.outline } else { pal.body };
                r.put(x + dx, y + dy, c);
                if !shadow {
                    r.put(x, y - 1, c);
                }
            }
        }
    }
}
