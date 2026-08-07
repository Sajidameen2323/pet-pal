//! What the creature decides to do, and the little physics model it lives in.
//!
//! The state machine is a strict priority ladder — being dragged beats an
//! alert, which beats sleeping, which beats being annoyed, which beats chasing
//! the cursor, which beats idle wandering — so behaviours can never fight.

use crate::config::Config;
use crate::platforms::{Ledge, World};
use crate::sprites::{Anim, SpriteSet};
use crate::win::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Walk,
    Run,
    Fall,
    Sleep,
    Annoyed,
    Drag,
    Alert,
    Sit,
}

impl State {
    fn anim(self) -> Anim {
        match self {
            State::Idle => Anim::Idle,
            State::Walk => Anim::Walk,
            State::Run => Anim::Run,
            State::Fall => Anim::Fall,
            State::Sleep => Anim::Sleep,
            State::Annoyed => Anim::Annoyed,
            State::Drag => Anim::Drag,
            State::Alert => Anim::Alert,
            State::Sit => Anim::Sit,
        }
    }
}

const GRAVITY: f32 = 1600.0; // px/s^2
const TERMINAL: f32 = 1400.0; // px/s
const AIR_DRAG: f32 = 1.1; // per second
const RUN_MULTIPLIER: f32 = 2.3;
/// Stop chasing once the cursor is within this many pixels horizontally...
const CHASE_DEADZONE: f32 = 26.0;
/// ...and don't set off again until it is this far away. The gap between the
/// two is what stops the pet oscillating around the cursor: with a single
/// threshold it overshoots by a pixel, flips facing, overshoots back, and
/// wiggles in place forever.
const CHASE_RESUME: f32 = 64.0;
/// Vertical distance beyond which the cursor counts as out of reach rather
/// than caught. One boundary for both, so the pet cannot flip-flop between
/// "sitting happily" and "looking up".
const CHASE_REACH_Y: f32 = 80.0;
const ALERT_MS: u32 = 2600;
/// Vertical slack when deciding whether a ledge still supports the pet.
const SUPPORT_TOL: i32 = 3;
/// A ledge at least this wide is comfortable enough to nap on.
const BED_WIDTH: i32 = 64;
/// Jump reach. Anything higher or further away is not worth attempting.
const JUMP_MAX_RISE: i32 = 280;
const JUMP_MAX_DX: i32 = 420;
/// Clear the target ledge by this much, so a slight misjudgement still lands.
const JUMP_CLEARANCE: f32 = 50.0;
/// Air drag eats some horizontal travel during the climb; bias the launch
/// speed to compensate.
const JUMP_DRAG_BIAS: f32 = 1.35;
/// A ledge must be at least this wide before a full-length sprint is worth it.
const SPRINT_MIN_RUN: i32 = 220;
/// Minimum gap between jump attempts. Without it a pet that keeps missing the
/// same ledge retries every tick and looks frantic.
const JUMP_COOLDOWN_MS: u32 = 1_100;
/// Below `threshold - HYSTERESIS` the pet stops being annoyed. Prevents
/// flickering when load hovers right at the limit.
const CPU_HYSTERESIS: f32 = 0.12;
/// Redraw cadence while the pet is actually moving (40 fps).
const MOVE_STEP_MS: u32 = 25;

pub struct Ctx<'a> {
    pub world: &'a World,
    pub cursor: (i32, i32),
    pub cpu_load: f32,
    pub idle_ms: u64,
    pub cfg: &'a Config,
    pub rng: &'a mut Rng,
}

pub struct Pet {
    /// Feet position: horizontal centre and the surface the feet rest on.
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// 1 = facing right, -1 = facing left.
    pub facing: i32,
    pub state: State,
    pub grounded: bool,

    pub frame: usize,
    frame_acc: u32,
    cur_anim: Anim,

    /// Time spent in the current state, and how long to stay before rethinking.
    state_ms: u32,
    hold_ms: u32,

    ledge: Option<Ledge>,
    /// Half the creature's visible width. It turns around this far from a ledge
    /// end, so the body never hangs off a window — or off the screen.
    margin: i32,
    /// Per-creature multiplier on the configured speed, so a mouse skitters and
    /// a Sith lord does not.
    speed_scale: f32,
    /// When set, walking off the end of a ledge is intentional rather than
    /// something to turn around at. Cleared on landing.
    dive: bool,
    /// Counts down after a jump; blocks the next attempt until it expires.
    jump_cooldown: u32,
    /// Sticky flag for the CPU hysteresis band.
    was_annoyed: bool,
    /// Set by "Go to sleep" in the tray menu. Keeps the pet asleep regardless
    /// of the idle timer, until something explicitly rouses it.
    forced_sleep: bool,
}

impl Pet {
    pub fn new(x: f32, y: f32) -> Pet {
        Pet {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            facing: -1,
            state: State::Fall,
            grounded: false,
            frame: 0,
            frame_acc: 0,
            cur_anim: Anim::Fall,
            state_ms: 0,
            hold_ms: 0,
            ledge: None,
            margin: 8,
            speed_scale: 1.0,
            dive: false,
            jump_cooldown: 0,
            was_annoyed: false,
            forced_sleep: false,
        }
    }

    /// Tell the pet how wide its *body* is on screen, as a fraction of the
    /// frame. Called whenever the sprite set or scale changes.
    ///
    /// It is a fraction rather than the full frame width because a trailing
    /// tail is not body: the mouse would otherwise stop a tail's length short
    /// of every ledge end.
    pub fn set_body_width(&mut self, window_width: i32, half_frac: f32) {
        self.margin = ((window_width as f32 * half_frac) as i32).max(4);
    }

    /// How fast this creature moves relative to the configured speed.
    pub fn set_speed_scale(&mut self, scale: f32) {
        self.speed_scale = scale.clamp(0.25, 4.0);
    }

    pub fn anim(&self) -> Anim {
        self.state.anim()
    }

    pub fn is_dragging(&self) -> bool {
        self.state == State::Drag
    }

    fn enter(&mut self, s: State, hold_ms: u32) {
        if self.state != s {
            self.state = s;
            self.frame = 0;
            self.frame_acc = 0;
            self.cur_anim = s.anim();
        }
        self.state_ms = 0;
        self.hold_ms = hold_ms;
    }

    // -- external events ---------------------------------------------------

    pub fn begin_drag(&mut self) {
        // Physically picking it up cancels a commanded nap.
        self.forced_sleep = false;
        self.enter(State::Drag, 0);
        self.grounded = false;
        self.ledge = None;
        self.vx = 0.0;
        self.vy = 0.0;
    }

    pub fn drag_to(&mut self, x: f32, y: f32) {
        // Remember the motion so releasing the mouse throws the pet.
        self.vx = x - self.x;
        self.vy = y - self.y;
        self.x = x;
        self.y = y;
    }

    pub fn end_drag(&mut self) {
        // The stored deltas are per-mouse-event; scale to a per-second velocity
        // and cap it so a violent flick cannot fling the pet across the desktop.
        self.vx = (self.vx * 22.0).clamp(-900.0, 900.0);
        self.vy = (self.vy * 22.0).clamp(-900.0, 900.0);
        self.enter(State::Fall, 0);
        self.grounded = false;
    }

    /// Perk up and look toward something that just happened at `at_x`.
    pub fn notice(&mut self, at_x: Option<i32>) {
        if self.state == State::Drag {
            return;
        }
        if let Some(tx) = at_x {
            self.facing = if (tx as f32) < self.x { -1 } else { 1 };
        }
        self.vx = 0.0;
        self.enter(State::Alert, ALERT_MS);
    }

    /// Send the pet to sleep on request, and keep it there.
    ///
    /// The flag is the whole point. Sleep is otherwise driven by the idle
    /// timer, and choosing this menu item *is* input — so without it the next
    /// tick sees a fresh idle time and wakes the pet straight back up.
    pub fn force_sleep(&mut self) {
        self.forced_sleep = true;
        if self.grounded {
            self.vx = 0.0;
            self.enter(State::Sleep, 0);
        }
        // Not grounded: the flag makes it settle down once it lands.
    }

    pub fn wake(&mut self) {
        self.forced_sleep = false;
        if self.state == State::Sleep {
            self.enter(State::Idle, 900);
        }
    }

    /// Asleep, or on its way to bed because it was told to be.
    pub fn is_sleeping(&self) -> bool {
        self.state == State::Sleep || self.forced_sleep
    }

    pub fn teleport(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.vx = 0.0;
        self.vy = 0.0;
        self.grounded = false;
        self.ledge = None;
        self.enter(State::Fall, 0);
    }

    // -- per-tick ----------------------------------------------------------

    pub fn update(&mut self, dt: u32, ctx: &mut Ctx, set: &SpriteSet) {
        self.state_ms = self.state_ms.saturating_add(dt);
        self.jump_cooldown = self.jump_cooldown.saturating_sub(dt);

        if self.state == State::Drag {
            self.animate(dt, set);
            return;
        }

        self.physics(dt, ctx);
        self.think(ctx);
        self.animate(dt, set);
    }

    fn physics(&mut self, dt: u32, ctx: &mut Ctx) {
        let dtf = dt as f32 / 1000.0;

        if self.grounded {
            // The window we were standing on may have moved, closed or been
            // covered since the last scan.
            match ctx
                .world
                .support_at(self.x as i32, self.y as i32, SUPPORT_TOL)
            {
                Some(l) => {
                    self.ledge = Some(l);
                    self.y = l.y as f32;
                }
                None => {
                    self.grounded = false;
                    self.ledge = None;
                    self.enter(State::Fall, 0);
                }
            }
        }

        if self.grounded {
            let speed = ctx.cfg.speed * self.speed_scale;
            self.vx = match self.state {
                State::Walk => self.facing as f32 * speed,
                State::Run => self.facing as f32 * speed * RUN_MULTIPLIER,
                _ => 0.0,
            };
            if self.vx != 0.0 {
                self.x += self.vx * dtf;
                self.confine_to_ledge();
            }
            return;
        }

        // Airborne.
        let prev_y = self.y;
        self.vy = (self.vy + GRAVITY * dtf).min(TERMINAL);
        self.vx *= 1.0 - (AIR_DRAG * dtf).min(1.0);
        self.x += self.vx * dtf;
        self.y += self.vy * dtf;

        if self.vy > 0.0 {
            // Swept test against the highest surface below where we started, so
            // fast falls cannot tunnel through a thin ledge.
            if let Some(l) = ctx.world.ground_below(self.x as i32, prev_y as i32) {
                if self.y >= l.y as f32 {
                    self.land_on(l);
                    return;
                }
            }
        }

        // Safety net: if we somehow left every ledge's span (a monitor was
        // unplugged, say), drop onto the nearest floor rather than forever.
        let mon = ctx.world.monitor_at(self.x as i32, self.y as i32);
        if self.y > mon.rect.bottom as f32 + 64.0 {
            self.x = (self.x as i32).clamp(mon.work.left + 32, mon.work.right - 32) as f32;
            self.y = mon.work.bottom as f32;
            self.land_on(Ledge {
                x0: mon.work.left,
                x1: mon.work.right,
                y: mon.work.bottom,
            });
        }
    }

    fn land_on(&mut self, l: Ledge) {
        self.y = l.y as f32;
        self.vy = 0.0;
        self.vx = 0.0;
        self.grounded = true;
        self.dive = false;
        self.ledge = Some(l);
        self.enter(State::Idle, 700);
        self.confine_to_ledge();
    }

    /// Keep the pet on its ledge, turning it around at the ends — unless it is
    /// deliberately diving after a cursor that is further down.
    fn confine_to_ledge(&mut self) {
        let Some(l) = self.ledge else { return };
        let lo = (l.x0 + self.margin) as f32;
        let hi = (l.x1 - self.margin) as f32;
        if lo >= hi {
            self.x = ((l.x0 + l.x1) / 2) as f32;
            return;
        }
        if self.x < lo || self.x > hi {
            if self.dive {
                // Step off into open air.
                self.grounded = false;
                self.ledge = None;
                self.vy = 0.0;
                self.enter(State::Fall, 0);
                return;
            }
            self.x = self.x.clamp(lo, hi);
            self.facing = -self.facing;
        }
    }

    /// A ledge overhead the pet could actually jump to, or `None`.
    ///
    /// Reach is measured from the *pet*, not from whatever it is chasing —
    /// otherwise a distant cursor would authorise an impossible leap.
    fn reachable_ledge(&mut self, ctx: &mut Ctx) -> Option<Ledge> {
        if !self.grounded || self.jump_cooldown > 0 {
            return None;
        }
        ctx.world.ledge_above(
            self.x as i32,
            self.y as i32,
            JUMP_MAX_RISE,
            JUMP_MAX_DX,
            self.margin * 2,
            ctx.rng.below(64),
        )
    }

    /// Launch toward a ledge above. Ballistic, not guided — a missed jump just
    /// becomes a fall, and the pet tries again after the cooldown.
    fn jump_to(&mut self, target: Ledge) {
        self.jump_cooldown = JUMP_COOLDOWN_MS;
        let rise = (self.y - target.y as f32).max(0.0) + JUMP_CLEARANCE;
        let v0 = (2.0 * GRAVITY * rise).sqrt().min(TERMINAL);
        let landing = (self.x as i32).clamp(target.x0 + self.margin, target.x1 - self.margin);
        let dx = landing as f32 - self.x;

        let time_to_apex = v0 / GRAVITY;
        self.vy = -v0;
        self.vx = (dx / time_to_apex * JUMP_DRAG_BIAS).clamp(-520.0, 520.0);
        if dx.abs() > 1.0 {
            self.facing = if dx < 0.0 { -1 } else { 1 };
        }
        self.grounded = false;
        self.ledge = None;
        self.enter(State::Fall, 0);
    }

    fn think(&mut self, ctx: &mut Ctx) {
        // Falling and being alerted run to completion.
        if self.state == State::Fall {
            return;
        }
        if self.state == State::Alert {
            if self.state_ms < self.hold_ms {
                return;
            }
            self.enter(State::Idle, 800);
        }

        let cfg = ctx.cfg;
        // Either the idle timer ran out, or the user asked for it directly.
        let sleepy = self.forced_sleep
            || ctx.idle_ms >= cfg.sleep_after_idle_secs.saturating_mul(1000);

        // Any input wakes the pet immediately -- unless it was told to sleep.
        if self.state == State::Sleep {
            if sleepy {
                return;
            }
            self.enter(State::Idle, 1200);
            return;
        }

        if sleepy {
            self.go_to_bed(ctx);
            return;
        }

        // CPU annoyance, with a hysteresis band so it doesn't strobe.
        let threshold = cfg.cpu_annoy_percent as f32 / 100.0;
        let annoyed = if self.was_annoyed {
            ctx.cpu_load >= threshold - CPU_HYSTERESIS
        } else {
            ctx.cpu_load >= threshold
        };
        self.was_annoyed = annoyed;
        if annoyed {
            if self.state != State::Annoyed {
                self.vx = 0.0;
                self.enter(State::Annoyed, 0);
            }
            return;
        }
        if self.state == State::Annoyed {
            self.enter(State::Idle, 600);
        }

        if cfg.chase_cursor {
            self.chase(ctx);
            return;
        }

        if self.state_ms >= self.hold_ms {
            self.wander(ctx);
        }
    }

    fn chase(&mut self, ctx: &mut Ctx) {
        let (cx, cy) = ctx.cursor;
        let dx = cx as f32 - self.x;

        let overhead = self.y - cy as f32;

        // Wider band to start moving than to stop, so the pet settles instead
        // of hunting around the target.
        let band = if self.state == State::Run {
            CHASE_DEADZONE
        } else {
            CHASE_RESUME
        };

        if dx.abs() <= band {
            self.dive = false;
            if overhead > CHASE_REACH_Y {
                // Directly underneath a cursor it cannot reach. Climb if there
                // is something to climb; otherwise stand and look up. Falling
                // through to Run here is what made it wiggle: `dx` is ~0, so
                // facing flipped on every tick.
                if let Some(l) = self.reachable_ledge(ctx) {
                    self.jump_to(l);
                }
                if self.state != State::Idle && self.state != State::Fall {
                    self.enter(State::Idle, 0);
                }
                return;
            }
            // Caught it — sit and look pleased until the cursor moves away.
            if self.state != State::Sit {
                self.enter(State::Sit, 0);
            }
            return;
        }

        // Worth moving for. Climb first if the cursor is well above us.
        if overhead > CHASE_REACH_Y {
            if let Some(l) = self.reachable_ledge(ctx) {
                self.jump_to(l);
                return;
            }
        }

        self.facing = if dx < 0.0 { -1 } else { 1 };
        // Only step off a ledge if the cursor is genuinely below us and there
        // is something down there; otherwise pace back and forth at the edge.
        self.dive = cy as f32 > self.y + 24.0
            && ctx
                .world
                .has_ledge_below(self.x as i32, self.y as i32, JUMP_MAX_DX);

        if self.state != State::Run {
            self.enter(State::Run, 0);
        }
    }

    /// Walk to a wide ledge, then curl up on it.
    fn go_to_bed(&mut self, ctx: &mut Ctx) {
        let on_a_bed = self.ledge.is_some_and(|l| l.width() >= BED_WIDTH);
        if on_a_bed {
            if self.state != State::Sleep {
                self.vx = 0.0;
                self.enter(State::Sleep, 0);
            }
            return;
        }

        match ctx.world.nearest_ledge(self.x as i32, self.y as i32) {
            Some(bed) => {
                let target = (bed.x0 + bed.x1) as f32 / 2.0;
                if (target - self.x).abs() < 8.0 {
                    self.enter(State::Sleep, 0);
                } else {
                    self.facing = if target < self.x { -1 } else { 1 };
                    if self.state != State::Walk {
                        self.enter(State::Walk, 0);
                    }
                }
            }
            // Nowhere better to go; sleep where we stand.
            None => self.enter(State::Sleep, 0),
        }
    }

    /// Pick the next idle behaviour.
    ///
    /// `cfg.roam` (0-100) is the single dial: it is the chance that this
    /// decision is to *move* rather than settle, it scales how often the pet
    /// tries to hop onto a window, and it stretches resting holds while
    /// shortening moving ones. At 0 the pet parks itself; at 100 it barely
    /// stops.
    fn wander(&mut self, ctx: &mut Ctx) {
        let roam = ctx.cfg.roam.min(100) as u32;

        // Hopping up onto a window is what makes the pet feel like it lives in
        // the desktop rather than on it, so try that first. Even a very calm
        // pet does it occasionally.
        let hop_chance = (4 + roam * 24 / 100).min(28);
        if ctx.rng.chance(hop_chance) {
            if let Some(l) = self.reachable_ledge(ctx) {
                self.jump_to(l);
                return;
            }
        }

        let drop_available =
            ctx.world
                .has_ledge_below(self.x as i32, self.y as i32, JUMP_MAX_DX);

        // Restlessness stretches or squeezes how long a choice is held, so a
        // calm pet takes long rests and a restless one changes its mind often.
        let rest_scale = 150 - roam;
        let move_scale = 55 + roam;
        let rng = &mut *ctx.rng;
        let hold = |rng: &mut Rng, lo: i32, hi: i32, scale: u32| {
            (rng.range(lo, hi) as u32 * scale / 100).max(250)
        };

        if rng.below(100) >= roam {
            // Settle: stand, sit, or turn on the spot to look around.
            match rng.below(100) {
                0..=54 => {
                    let d = hold(rng, 1400, 4200, rest_scale);
                    self.enter(State::Idle, d);
                }
                55..=87 => {
                    let d = hold(rng, 2200, 6000, rest_scale);
                    self.enter(State::Sit, d);
                }
                _ => {
                    self.facing = -self.facing;
                    let d = hold(rng, 500, 1200, rest_scale);
                    self.enter(State::Idle, d);
                }
            }
            return;
        }

        // Move. A restless creature is more likely to pick the long sprint.
        let sprint_chance = 12 + roam * 30 / 100;
        if ctx.rng.chance(sprint_chance) && self.sprint_across(ctx) {
            return;
        }
        let rng = &mut *ctx.rng;

        match rng.below(100) {
            0..=63 => {
                if rng.chance(45) {
                    self.facing = -self.facing;
                }
                let d = hold(rng, 1200, 3600, move_scale);
                self.enter(State::Walk, d);
            }
            64..=81 => {
                // Deliberately walk off the edge and drop to whatever is below
                // — but only if there actually is something below.
                self.dive = drop_available;
                let d = hold(rng, 900, 2000, move_scale);
                self.enter(State::Walk, d);
            }
            _ => {
                // A dash, for personality.
                if rng.chance(50) {
                    self.facing = -self.facing;
                }
                let d = hold(rng, 500, 1400, move_scale);
                self.enter(State::Run, d);
            }
        }
    }

    /// Sprint from one end of the current ledge to the other.
    ///
    /// The generic "dash" holds Run for a second or so, which on a wide surface
    /// is a twitch rather than a run. This aims at the far end and holds for
    /// however long the crossing actually takes, so the creature covers the
    /// whole taskbar in one go.
    fn sprint_across(&mut self, ctx: &mut Ctx) -> bool {
        let Some(l) = self.ledge else { return false };
        let (lo, hi) = (l.x0 + self.margin, l.x1 - self.margin);
        if hi - lo < SPRINT_MIN_RUN {
            return false;
        }
        // Head for whichever end is further away.
        let target = if self.x - lo as f32 > hi as f32 - self.x {
            lo
        } else {
            hi
        };
        let distance = (target as f32 - self.x).abs();
        let speed = (ctx.cfg.speed * self.speed_scale * RUN_MULTIPLIER).max(1.0);
        let ms = (distance / speed * 1000.0) as u32;

        self.facing = if (target as f32) < self.x { -1 } else { 1 };
        self.enter(State::Run, ms.clamp(700, 20_000));
        true
    }

    fn animate(&mut self, dt: u32, set: &SpriteSet) {
        let a = self.anim();
        if a != self.cur_anim {
            self.cur_anim = a;
            self.frame = 0;
            self.frame_acc = 0;
        }
        let ms = set.clip(a).frame_ms.max(16);
        self.frame_acc += dt;
        let advance = (self.frame_acc / ms) as usize;
        if advance > 0 {
            self.frame_acc %= ms;
            self.frame = (self.frame + advance) % set.frame_count(a).max(1);
        }
    }

    /// How long the app may sleep before this pet needs redrawing.
    ///
    /// A stationary pet only needs a repaint when its animation frame changes,
    /// which is why an idle creature costs almost nothing.
    pub fn next_wake_ms(&self, set: &SpriteSet) -> u32 {
        if self.state == State::Drag {
            return 16;
        }
        // Check the state as well as the velocity: on the first tick after
        // entering Walk, `think` has chosen the state but `physics` has not yet
        // set `vx`, and reading velocity alone would stall the start of a step.
        if !self.grounded
            || self.vx != 0.0
            || matches!(self.state, State::Walk | State::Run | State::Fall)
        {
            return MOVE_STEP_MS;
        }
        let ms = set.clip(self.anim()).frame_ms.max(16);
        ms.saturating_sub(self.frame_acc).max(8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::Monitor;
    use crate::sprites::{self, Kind, Palette};
    use windows_sys::Win32::Foundation::RECT;

    const FLOOR_Y: i32 = 960;

    fn world() -> World {
        World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1600, bottom: 1000 },
                work: RECT { left: 0, top: 0, right: 1600, bottom: FLOOR_Y },
            }],
            // A single wide floor and nothing to climb.
            vec![Ledge { x0: 0, x1: 1600, y: FLOOR_Y }],
        )
    }

    /// Drop the pet onto the floor and settle it.
    fn grounded_pet(set: &SpriteSet, cfg: &Config) -> (Pet, Rng) {
        let mut pet = Pet::new(800.0, 900.0);
        pet.set_body_width(96, Kind::Pal.body_half_frac());
        let mut rng = Rng::new();
        let w = world();
        for _ in 0..40 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (800, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, set);
        }
        assert!(pet.grounded, "pet should have landed");
        (pet, rng)
    }

    fn chase_config() -> Config {
        // Built by mutation rather than struct update syntax: Config keeps a
        // private field that struct-update would have to name.
        let mut cfg = Config::default();
        cfg.chase_cursor = true;
        // Never sleep, never get annoyed: isolate the chase behaviour.
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;
        cfg
    }

    /// Regression: with the cursor parked directly overhead and nothing to
    /// climb, the pet used to skip the horizontal deadzone (it was gated on
    /// vertical proximity), fall through to Run with dx ~ 0, and flip facing
    /// every tick — visibly wiggling in place.
    #[test]
    fn does_not_wiggle_under_an_unreachable_overhead_cursor() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = chase_config();
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        let start_x = pet.x;
        let start_facing = pet.facing;
        // Far above the floor, well out of jumping reach, exactly overhead.
        let cursor = (start_x as i32, FLOOR_Y - 500);

        let mut flips = 0;
        let mut prev_facing = pet.facing;
        for _ in 0..400 {
            let mut ctx = Ctx {
                world: &w,
                cursor,
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            if pet.facing != prev_facing {
                flips += 1;
                prev_facing = pet.facing;
            }
        }

        assert_eq!(flips, 0, "facing flipped {flips} times while standing still");
        assert_eq!(pet.facing, start_facing);
        assert!(
            (pet.x - start_x).abs() < 1.0,
            "drifted {} px under a stationary cursor",
            (pet.x - start_x).abs()
        );
        assert_ne!(pet.state, State::Run, "should not be running on the spot");
    }

    /// Fraction of ticks spent actually moving, for a given restlessness.
    fn moving_fraction(roam: u8) -> f32 {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = Config::default();
        cfg.roam = roam;
        // Isolate wandering from sleeping, annoyance and chasing.
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;
        cfg.chase_cursor = false;

        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();
        let (mut moving, mut total) = (0u32, 0u32);
        for _ in 0..6000 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (10_000, 10_000), // far away and irrelevant
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            total += 1;
            if matches!(pet.state, State::Walk | State::Run) {
                moving += 1;
            }
        }
        moving as f32 / total as f32
    }

    /// Regression: "Go to sleep" used to last a single tick. Sleep is driven
    /// by the idle timer, and choosing the menu item *is* input, so the very
    /// next tick saw idle_ms ~ 0 and woke the pet straight back up.
    #[test]
    fn commanded_sleep_survives_fresh_input() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = chase_config(); // long sleep timer, so only the command applies
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        pet.force_sleep();
        assert_eq!(pet.state, State::Sleep, "should be asleep immediately");

        // Twenty seconds of ticks with the user constantly at the keyboard.
        for _ in 0..800 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (400, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0, // input every single tick
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
        }
        assert_eq!(pet.state, State::Sleep, "commanded sleep must not time out");
        assert!(pet.is_sleeping());

        // "Wake up" ends it, and it stays awake.
        pet.wake();
        assert_ne!(pet.state, State::Sleep);
        assert!(!pet.is_sleeping());
        for _ in 0..200 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (400, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
        }
        assert_ne!(pet.state, State::Sleep, "should not crawl back to bed");
    }

    /// Picking the pet up cancels a commanded nap.
    #[test]
    fn dragging_cancels_commanded_sleep() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = chase_config();
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        pet.force_sleep();
        assert!(pet.is_sleeping());
        pet.begin_drag();
        pet.drag_to(500.0, 300.0);
        pet.end_drag();
        assert!(!pet.is_sleeping(), "picking it up should rouse it");

        for _ in 0..400 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (400, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
        }
        assert_ne!(pet.state, State::Sleep);
    }

    /// `roam` is the one dial for restlessness; the ends of its range must
    /// actually differ, or the setting is decoration.
    #[test]
    fn roam_controls_how_much_it_moves() {
        let calm = moving_fraction(0);
        let normal = moving_fraction(45);
        let busy = moving_fraction(100);

        assert!(calm < 0.15, "a pet set to never roam moved on {calm:.2} of ticks");
        assert!(busy > 0.70, "a pet set to always roam moved on only {busy:.2} of ticks");
        assert!(
            calm < normal && normal < busy,
            "roam should be monotonic: {calm:.2} / {normal:.2} / {busy:.2}"
        );
    }

    /// The deadzone must not stop it chasing a cursor that is genuinely away.
    #[test]
    fn still_chases_a_distant_cursor() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = chase_config();
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        let start_x = pet.x;
        for _ in 0..200 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (200, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
        }
        assert!(pet.x < start_x - 100.0, "did not chase left: {start_x} -> {}", pet.x);
        assert_eq!(pet.facing, -1);
    }

    /// Once it arrives it should settle, not hunt back and forth across the
    /// cursor. This is what the resume band buys us.
    #[test]
    fn settles_when_it_reaches_the_cursor() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = chase_config();
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        let target = 400;
        let run = |pet: &mut Pet, rng: &mut Rng, n: usize| {
            for _ in 0..n {
                let mut ctx = Ctx {
                    world: &w,
                    cursor: (target, FLOOR_Y),
                    cpu_load: 0.0,
                    idle_ms: 0,
                    cfg: &cfg,
                    rng,
                };
                pet.update(25, &mut ctx, &set);
            }
        };

        run(&mut pet, &mut rng, 400); // walk over
        let settled = pet.x;
        assert!(
            (settled - target as f32).abs() <= CHASE_RESUME,
            "stopped {} px from the cursor",
            (settled - target as f32).abs()
        );

        run(&mut pet, &mut rng, 200); // and stay put
        assert!(
            (pet.x - settled).abs() < 1.0,
            "drifted {} px after arriving",
            (pet.x - settled).abs()
        );
    }
}
