//! What the creature decides to do, and the little physics model it lives in.
//!
//! The state machine is a strict priority ladder — being dragged beats an
//! alert, which beats sleeping, which beats being annoyed, which beats chasing
//! the cursor, which beats idle wandering — so behaviours can never fight.

use windows_sys::Win32::Foundation::HWND;

use crate::config::Config;
use crate::platforms::{Ledge, Wall, World};
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
    /// Scaling the vertical edge of a window.
    Climb,
    /// Showing off: a little trick, done for its own sake.
    Play,
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
            State::Climb => Anim::Climb,
            State::Play => Anim::Play,
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
/// The shortest journey worth making. Below this the creature is shuffling,
/// not walking, so it rests instead — which is what a real animal on a narrow
/// perch does.
const MIN_TRAVEL: f32 = 48.0;
/// A surface with less usable width than this is a perch, not somewhere to
/// roam. The creature mostly sits on one, because every "move" it is asked for
/// there is a couple of steps and a turn — which is legal by the rules and
/// reads as an animal trapped in a loop.
const CRAMPED_SPAN: i32 = 240;
/// How far the creature will deliberately drop to reach a lower surface.
/// Further than this and it walks to an edge and falls instead.
const HOP_MAX_DROP: i32 = 900;
/// A hop down only needs to clear the lip of the current ledge, not gain
/// height, so it pops up far less than a jump.
const HOP_DOWN_CLEARANCE: f32 = 18.0;
/// How long the creature explores a newly reached surface before it will
/// consider moving on. Without it, landing is immediately followed by another
/// decision, and since the direction bias favours going down when high up, a
/// climb ends in an instant hop straight back — a pointless yo-yo.
const SETTLE_BASE_MS: u32 = 6_000;
/// Even the most restless creature looks at its new surroundings this long.
const SETTLE_MIN_MS: u32 = 1_500;
/// How fast the creature scales a window edge, px/s.
const CLIMB_SPEED: f32 = 165.0;
/// How far it will walk sideways to reach an edge worth climbing.
const CLIMB_SEEK_DX: i32 = 500;
/// Close enough to take hold of the edge.
const CLIMB_GRAB_DX: i32 = 14;
/// An edge has to lead at least this much higher to be worth climbing rather
/// than jumping.
const CLIMB_MIN_GAIN: i32 = 90;
/// Give up on a climb that is taking absurdly long — the window moved, or the
/// geometry changed under us.
const CLIMB_TIMEOUT_MS: u32 = 20_000;
/// After abandoning an approach, leave the whole idea alone for a while. The
/// edge that could not be reached is usually still the nearest one, so without
/// this the next decision picks it straight back up.
const LEAVE_COOLDOWN_MS: u32 = 25_000;
/// And give up on *reaching* an edge that never gets closer. A creature pinned
/// against something it cannot walk past would otherwise re-aim at the same
/// edge forever, never rolling for anything else to do.
const CLIMB_APPROACH_TIMEOUT_MS: u32 = 20_000;
/// How often the creature asks itself whether to leave the surface it is on.
///
/// On its own clock rather than folded into the next wandering decision: a
/// walk with a destination can run for ten seconds, and gating the question on
/// the end of one made climbing take three times longer.
const LEAVE_CHECK_MS: u32 = 1_200;
/// Minimum gap between jump attempts. Without it a pet that keeps missing the
/// same ledge retries every tick and looks frantic.
const JUMP_COOLDOWN_MS: u32 = 1_100;
/// Below `threshold - HYSTERESIS` the pet stops being annoyed. Prevents
/// flickering when load hovers right at the limit.
const CPU_HYSTERESIS: f32 = 0.12;
/// Redraw cadence while the pet is actually moving (40 fps).
const MOVE_STEP_MS: u32 = 25;
/// How long a trick is held. Long enough for a couple of loops of the built-in
/// eight-frame clip, so it reads as a routine rather than a twitch.
const PLAY_MS: u32 = 1_600;
/// Odds of a rest becoming a trick at `roam = 0`; restlessness adds to it.
const TRICK_BASE_CHANCE: u32 = 4;
/// Odds that poking the creature gets a trick rather than the usual delight.
const POKE_TRICK_CHANCE: u32 = 45;

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
    /// The edge currently being scaled.
    climbing: Option<Wall>,
    /// An edge chosen but not yet reached. Held across decisions so the walk
    /// over to it is a journey rather than a coin flip every few seconds.
    climb_goal: Option<Wall>,
    climb_goal_ms: u32,
    /// Time spent on the current surface since arriving on it.
    settled_ms: u32,
    /// While hopping *down*, the surface it launched from must be ignored or
    /// the small upward pop lands it straight back where it started. Only
    /// ledges at or below this y count as ground until it touches down.
    land_below: Option<i32>,
    /// Time since the last "should I move on?" check.
    leave_acc: u32,
    /// Counts down after an abandoned approach; blocks further attempts.
    leave_cooldown: u32,
    /// Sticky flag for the CPU hysteresis band.
    was_annoyed: bool,
    /// Top-left of the window the creature is standing on, as of the last tick.
    /// The delta against the window's current position is how far it has to be
    /// carried; `None` means it is not on a window at all. See [`Pet::ride`].
    ride_from: Option<(i32, i32)>,
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
            climbing: None,
            climb_goal: None,
            climb_goal_ms: 0,
            settled_ms: 0,
            land_below: None,
            leave_acc: 0,
            leave_cooldown: 0,
            was_annoyed: false,
            ride_from: None,
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
        // A commanded nap survives being carried: only "Wake up" ends it, so
        // the pet curls back up wherever you put it down.
        self.land_below = None;
        self.climbing = None;
        self.climb_goal = None;
        self.enter(State::Drag, 0);
        self.grounded = false;
        self.ledge = None;
        self.ride_from = None;
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
    ///
    /// A commanded nap outranks every one of these: an app opening, a reminder
    /// firing or a poke must not drag the pet out of bed. The idle-timer nap is
    /// left alone — that one is *meant* to break on the first sign of life.
    pub fn notice(&mut self, at_x: Option<i32>) {
        if self.state == State::Drag || self.forced_sleep {
            return;
        }
        if let Some(tx) = at_x {
            self.facing = if (tx as f32) < self.x { -1 } else { 1 };
        }
        self.vx = 0.0;
        self.enter(State::Alert, ALERT_MS);
    }

    /// Poked by the user: usually delighted, sometimes a whole trick.
    ///
    /// Clicking the creature is the most direct thing anyone does to it, and a
    /// reaction that is identical every single time stops registering as one.
    /// A trick needs the ground under it, so a poke taken mid-air falls back to
    /// the plain reaction. A commanded nap outranks both.
    pub fn poke(&mut self, rng: &mut Rng) {
        if self.state == State::Drag || self.forced_sleep {
            return;
        }
        if self.grounded && rng.chance(POKE_TRICK_CHANCE) {
            self.vx = 0.0;
            self.enter(State::Play, PLAY_MS);
            return;
        }
        self.notice(None);
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
        self.ride_from = None;
        self.land_below = None;
        self.climbing = None;
        self.climb_goal = None;
        self.enter(State::Fall, 0);
    }

    // -- per-tick ----------------------------------------------------------

    /// Travel with the window underfoot.
    ///
    /// `where_is` reports a window's current top-left, or `None` once it is
    /// gone. Called once per tick, before the simulation.
    ///
    /// Without this the creature keeps its screen coordinates while a window
    /// slides out from under it: drag a window sideways and it is left standing
    /// in mid-air, drag it down and the 3px support tolerance drops it. The
    /// ledge scan cannot fix that on its own — it runs a few times a second,
    /// which is far too coarse to follow a drag — so the window is asked
    /// directly, every tick, and the creature is moved by the difference.
    ///
    /// Returns whether it is currently riding, which tells [`Self::physics`] to
    /// trust the ledge it is holding over the scan's staler copy.
    pub fn ride(&mut self, world: &World, where_is: impl Fn(HWND) -> Option<(i32, i32)>) -> bool {
        // Only a grounded creature on a window rides; being carried, airborne
        // or on a monitor floor are all "stay where you are".
        let owner = match (self.grounded, self.state) {
            (true, s) if s != State::Drag && s != State::Climb => {
                self.ledge.and_then(|l| l.window())
            }
            _ => None,
        };
        let Some(owner) = owner else {
            self.ride_from = None;
            return false;
        };
        // A window whose edge the scan no longer offers has been covered by
        // something else. Stop riding and let the usual support check drop the
        // creature, rather than leaving it standing on a hidden title bar.
        let Some(now) = where_is(owner).filter(|_| world.owns_ledge(owner)) else {
            self.ride_from = None;
            return false;
        };

        if let Some(prev) = self.ride_from {
            let (dx, dy) = (now.0 - prev.0, now.1 - prev.1);
            if dx != 0 || dy != 0 {
                self.x += dx as f32;
                self.y += dy as f32;
                if let Some(l) = self.ledge.as_mut() {
                    l.x0 += dx;
                    l.x1 += dx;
                    l.y += dy;
                }
            }
        }
        self.ride_from = Some(now);
        true
    }

    pub fn update(&mut self, dt: u32, ctx: &mut Ctx, set: &SpriteSet) {
        self.state_ms = self.state_ms.saturating_add(dt);
        self.jump_cooldown = self.jump_cooldown.saturating_sub(dt);
        if self.climb_goal.is_some() {
            self.climb_goal_ms = self.climb_goal_ms.saturating_add(dt);
        }
        if self.grounded {
            self.settled_ms = self.settled_ms.saturating_add(dt);
            self.leave_acc = self.leave_acc.saturating_add(dt);
        }
        self.leave_cooldown = self.leave_cooldown.saturating_sub(dt);

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

        if self.state == State::Climb {
            self.climb_step(dtf, ctx);
            return;
        }

        if self.grounded && self.ride_from.is_some() {
            // Riding a window this tick: `ride` already confirmed the window is
            // there and moved the creature and its ledge with it. The scan's
            // copy of that ledge is up to a refresh interval out of date, and
            // adopting it here would drag the creature back to where the window
            // used to be — a stutter while dragging, and a fall the moment the
            // window outruns the 3px support tolerance.
            if let Some(l) = self.ledge {
                self.y = l.y as f32;
            }
        } else if self.grounded {
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
                    self.ride_from = None;
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
            let probe = self
                .land_below
                .map_or(prev_y as i32, |floor| (prev_y as i32).max(floor));
            if let Some(l) = ctx.world.ground_below(self.x as i32, probe) {
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
            self.land_on(Ledge::at(
                mon.work.left,
                mon.work.right,
                mon.work.bottom,
            ));
        }
    }

    fn land_on(&mut self, l: Ledge) {
        // Arriving somewhere new restarts the settle clock.
        self.settled_ms = 0;
        self.y = l.y as f32;
        self.vy = 0.0;
        self.vx = 0.0;
        self.grounded = true;
        self.dive = false;
        self.land_below = None;
        self.climbing = None;
        // Arriving somewhere new drops any half-finished plan. Keeping it would
        // let the pet skip the settle gate it just reset, land and immediately
        // set off again — the yo-yo that gate exists to stop.
        self.climb_goal = None;
        self.ledge = Some(l);
        self.ride_from = None;
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
                self.ride_from = None;
                self.vy = 0.0;
                self.enter(State::Fall, 0);
                return;
            }
            self.x = self.x.clamp(lo, hi);
            self.facing = -self.facing;
            // Re-decide here, rather than spending the rest of this walk
            // drifting back the way we came.
            //
            // A purposeful walk that reaches the end of its surface has
            // finished, and the creature is now standing at the closest point
            // it can occupy to whatever it was heading for — which is exactly
            // where a climb gets taken hold of. Without this it never *ends* a
            // walk at the edge: it passes through the point it needed, is
            // turned around, and the next decision is taken a body's width
            // further away, so it aims at the same target again. Forever.
            //
            // Measured on a 2560x1440 screen with a window 40px in from the
            // edge: the creature oscillated between x=54 and x=68 — a 14px
            // shuffle — 353 times in three minutes.
            self.hold_ms = 0;
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

    /// Pick somewhere to hop, up or down, and go.
    ///
    /// Direction is biased by how high the creature already is: only ever
    /// jumping up strands it on the topmost window, and only ever dropping
    /// parks it on the taskbar. Weighting by height makes it circulate.
    fn try_hop(&mut self, ctx: &mut Ctx) -> bool {
        if !self.grounded || self.jump_cooldown > 0 {
            return false;
        }
        let (x, y) = (self.x as i32, self.y as i32);
        let min_w = self.margin * 2;
        let up = ctx
            .world
            .ledge_above(x, y, JUMP_MAX_RISE, JUMP_MAX_DX, min_w, ctx.rng.below(64));
        let down = ctx
            .world
            .ledge_below(x, y, HOP_MAX_DROP, JUMP_MAX_DX, min_w, ctx.rng.below(64));

        // 0 at the top of the work area, 1 at the bottom.
        let m = ctx.world.monitor_at(x, y);
        let span = (m.work.bottom - m.work.top).max(1) as f32;
        let height = ((y - m.work.top) as f32 / span).clamp(0.0, 1.0);
        let prefer_down = (25.0 + 50.0 * (1.0 - height)) as u32;

        match (up, down) {
            (Some(u), Some(d)) => {
                if ctx.rng.chance(prefer_down) {
                    self.leap_to(d, HOP_DOWN_CLEARANCE);
                } else {
                    self.leap_to(u, JUMP_CLEARANCE);
                }
                true
            }
            (Some(u), None) => {
                self.leap_to(u, JUMP_CLEARANCE);
                true
            }
            (None, Some(d)) => {
                self.leap_to(d, HOP_DOWN_CLEARANCE);
                true
            }
            (None, None) => false,
        }
    }

    /// Launch at a ledge, up or down. Ballistic, not guided — a missed leap is
    /// just a fall, and the pet tries again after the cooldown.
    ///
    /// One arc covers both directions. Solving the full trajectory rather than
    /// only the rise is what makes a hop *down* look deliberate: the pet pops
    /// up by `clearance`, then travels far enough sideways during the whole
    /// descent to actually land on the target, instead of dribbling off the
    /// edge and dropping straight down.
    fn leap_to(&mut self, target: Ledge, clearance: f32) {
        self.jump_cooldown = JUMP_COOLDOWN_MS;

        // Positive when the target is below us.
        let drop = target.y as f32 - self.y;
        let rise = (-drop).max(0.0) + clearance;
        let v0 = (2.0 * GRAVITY * rise).sqrt().min(TERMINAL);

        // Time to reach the target height: 0.5*g*t^2 - v0*t - drop = 0.
        let disc = (v0 * v0 + 2.0 * GRAVITY * drop).max(0.0);
        let flight = ((v0 + disc.sqrt()) / GRAVITY).max(0.05);

        let landing = (self.x as i32).clamp(target.x0 + self.margin, target.x1 - self.margin);
        let dx = landing as f32 - self.x;

        self.vy = -v0;
        self.vx = (dx / flight * JUMP_DRAG_BIAS).clamp(-520.0, 520.0);
        if dx.abs() > 1.0 {
            self.facing = if dx < 0.0 { -1 } else { 1 };
        }
        // A hop down has to fall *through* the surface it is standing on. The
        // small upward pop would otherwise re-satisfy the landing test against
        // that same ledge on the way back down, and the pet would never leave.
        // A jump up needs no such thing, and should still be able to land back
        // here if it misjudges the leap.
        self.land_below = (drop > 0.0).then_some(self.y as i32 + 8);
        self.grounded = false;
        self.ledge = None;
        self.ride_from = None;
        self.enter(State::Fall, 0);
    }

    /// Take hold of an edge and start going up.
    fn start_climb(&mut self, w: Wall) {
        self.climbing = Some(w);
        self.x = w.x as f32;
        // Face the window it belongs to, so it looks like it is holding on.
        self.facing = w.inward;
        self.vx = 0.0;
        self.vy = 0.0;
        self.grounded = false;
        self.ledge = None;
        self.ride_from = None;
        self.enter(State::Climb, 0);
    }

    /// One tick of climbing: straight up the edge, then step onto the top.
    fn climb_step(&mut self, dtf: f32, ctx: &mut Ctx) {
        let Some(w) = self.climbing else {
            self.let_go();
            return;
        };
        // Windows move and close. If the edge is gone, so is the grip.
        if self.state_ms > CLIMB_TIMEOUT_MS
            || ctx.world.wall_at(w.x, self.y as i32).is_none()
        {
            self.let_go();
            return;
        }

        self.y -= CLIMB_SPEED * dtf;
        self.x = w.x as f32;

        if self.y <= w.y_top as f32 {
            // Over the lip and onto the top surface, a body's width inside so
            // it is standing on the window rather than balanced on its corner.
            self.y = w.y_top as f32;
            self.x = (w.x + w.inward * self.margin) as f32;
            match ctx.world.support_at(self.x as i32, self.y as i32, 8) {
                Some(l) => {
                    self.climbing = None;
                    self.land_on(l);
                }
                // The top edge was clipped away by another window covering it.
                None => self.let_go(),
            }
        }
    }

    fn let_go(&mut self) {
        self.climbing = None;
        self.vx = 0.0;
        self.vy = 0.0;
        self.grounded = false;
        self.enter(State::Fall, 0);
    }

    /// Head for a climbable edge when nothing is within jumping reach.
    ///
    /// The chosen edge is remembered. It used to be inferred from the walk
    /// instead — give the walk roughly the time needed to cover the distance,
    /// and when it expires the creature is beside the edge. That works right up
    /// until the walk is short enough that `wander` gets another turn first, and
    /// then the next decision is a fresh roll that often sends it back the other
    /// way. Approaching an edge 300px off became a random walk: on a plain
    /// one-window desktop it took between 37s and 4m40s to make a climb that
    /// should take about five seconds of walking.
    fn seek_climb(&mut self, ctx: &mut Ctx) -> bool {
        if !self.grounded {
            return false;
        }
        let Some(w) = ctx.world.wall_near(
            self.x as i32,
            self.y as i32,
            CLIMB_SEEK_DX,
            CLIMB_MIN_GAIN,
            self.reach(),
            CLIMB_GRAB_DX,
        ) else {
            return false;
        };
        // Only worth setting out for if it is on the surface we are standing on.
        if (w.x as f32 - self.x).abs() > CLIMB_GRAB_DX as f32
            && !self.ledge.is_some_and(|l| l.holds(w.x))
        {
            return false;
        }
        self.climb_goal = Some(w);
        self.climb_goal_ms = 0;
        self.approach_wall(w, ctx)
    }

    /// The span of x the creature can stand on, on its current surface.
    ///
    /// It keeps half a body width from each end, so this is narrower than the
    /// surface. Anything outside it is somewhere the creature can look at but
    /// never occupy.
    fn reach(&self) -> (i32, i32) {
        match self.ledge {
            Some(l) => {
                let (lo, hi) = (l.x0 + self.margin, l.x1 - self.margin);
                if lo <= hi { (lo, hi) } else { ((l.x0 + l.x1) / 2, (l.x0 + l.x1) / 2) }
            }
            None => (i32::MIN / 2, i32::MAX / 2),
        }
    }

    /// Carry on toward an edge already chosen, if it is still worth reaching.
    ///
    /// Everything that could have changed underneath the plan is rechecked
    /// here rather than trusted: the window may have moved, closed or been
    /// resized, the pet may have been knocked onto a different surface, and the
    /// user may have turned window-hopping off mid-walk.
    fn resume_climb(&mut self, ctx: &mut Ctx) -> bool {
        let Some(goal) = self.climb_goal else {
            return false;
        };
        let reach = self.reach();
        let stale = !ctx.cfg.jump_between_windows
            || !self.grounded
            || self.climb_goal_ms > CLIMB_APPROACH_TIMEOUT_MS
            // Windows move; an edge that was reachable from the old surface
            // may not be from this one.
            || (goal.x.clamp(reach.0, reach.1) - goal.x).abs() > CLIMB_GRAB_DX
            || ctx.world.wall_at(goal.x, self.y as i32).is_none()
            || !(self.ledge.is_some_and(|l| l.holds(goal.x))
                || (goal.x as f32 - self.x).abs() <= CLIMB_GRAB_DX as f32);
        if stale {
            // Whatever made it stale, do not immediately choose it again.
            if self.climb_goal_ms > CLIMB_APPROACH_TIMEOUT_MS {
                self.leave_cooldown = LEAVE_COOLDOWN_MS;
            }
            self.climb_goal = None;
            return false;
        }
        self.approach_wall(goal, ctx)
    }

    /// Walk the remaining distance to `w`, or take hold if already there.
    fn approach_wall(&mut self, w: Wall, ctx: &mut Ctx) -> bool {
        let dx = w.x as f32 - self.x;
        if dx.abs() <= CLIMB_GRAB_DX as f32 {
            self.climb_goal = None;
            self.start_climb(w);
            return true;
        }
        self.facing = if dx < 0.0 { -1 } else { 1 };
        let speed = (ctx.cfg.speed * self.speed_scale).max(1.0);
        let ms = (dx.abs() / speed * 1000.0) as u32;
        self.enter(State::Walk, ms.clamp(300, 12_000));
        true
    }

    fn think(&mut self, ctx: &mut Ctx) {
        // Falling, climbing and being alerted all run to completion.
        if self.state == State::Fall || self.state == State::Climb {
            return;
        }
        // A reaction and a trick both run to the end. Half a trick reads as the
        // creature thinking better of it.
        if self.state == State::Alert || self.state == State::Play {
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

        // Asked on its own cadence, so a long stroll does not postpone it.
        if self.consider_leaving(ctx) {
            return;
        }

        if self.state_ms >= self.hold_ms {
            self.wander(ctx);
        }
    }

    /// Roll for hopping or climbing off the current surface.
    ///
    /// Deliberately allowed to interrupt a walk in progress: arriving somewhere
    /// new is more interesting than finishing a stroll, and the settle gate
    /// already stops it happening the moment the creature lands.
    fn consider_leaving(&mut self, ctx: &mut Ctx) -> bool {
        if !self.grounded
            || !ctx.cfg.jump_between_windows
            || self.climb_goal.is_some()
            || self.leave_cooldown > 0
        {
            return false;
        }
        if self.leave_acc < LEAVE_CHECK_MS {
            return false;
        }
        self.leave_acc = 0;
        let roam = ctx.cfg.roam.min(100) as u32;
        let settle_needed = (SETTLE_BASE_MS * (140 - roam) / 100).max(SETTLE_MIN_MS);
        if self.settled_ms < settle_needed {
            return false;
        }
        let hop_chance = (4 + roam * 24 / 100).min(28);
        if !ctx.rng.chance(hop_chance) {
            return false;
        }
        self.try_hop(ctx) || self.seek_climb(ctx)
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
                    self.leap_to(l, JUMP_CLEARANCE);
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
                self.leap_to(l, JUMP_CLEARANCE);
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
        // An edge already chosen outranks a fresh roll. This is the whole
        // difference between walking to a window and drifting toward one.
        if self.resume_climb(ctx) {
            return;
        }

        let roam = ctx.cfg.roam.min(100) as u32;

        // Hopping and climbing are handled by `consider_leaving`, on its own
        // clock. What is left here is stepping off an edge, which shares the
        // same settle gate: otherwise the creature lands, pauses, and walks
        // straight back off, which is the yo-yo that gate exists to stop.
        //
        // Look around the new surface before planning the next move. A
        // restless creature gets bored of it sooner.
        let settle_needed =
            (SETTLE_BASE_MS * (140 - roam) / 100).max(SETTLE_MIN_MS);
        let may_leave = ctx.cfg.jump_between_windows && self.settled_ms >= settle_needed;


        // Stepping off an edge is a way of leaving too, and has to wait out the
        // same settle — otherwise the creature lands, idles for a moment and
        // walks straight back off, which looks exactly like the yo-yo the hop
        // gate was added to stop.
        let drop_available = may_leave
            && ctx
                .world
                .has_ledge_below(self.x as i32, self.y as i32, JUMP_MAX_DX);

        // Restlessness stretches or squeezes how long a choice is held, so a
        // calm pet takes long rests and a restless one changes its mind often.
        let move_scale = 55 + roam;

        // How much room there is decides how often it bothers moving at all.
        let cramped = self
            .ledge
            .is_some_and(|l| l.x1 - l.x0 - 2 * self.margin < CRAMPED_SPAN);
        let move_chance = if cramped { roam / 4 } else { roam };
        if ctx.rng.below(100) >= move_chance {
            self.rest(ctx);
            return;
        }

        // Move. A restless creature is more likely to pick the long sprint.
        let sprint_chance = 12 + roam * 30 / 100;
        if ctx.rng.chance(sprint_chance) && self.sprint_across(ctx) {
            return;
        }

        let choice = ctx.rng.below(100);
        let moved = match choice {
            // Deliberately walk off the edge and drop to whatever is below —
            // but only if there actually is something below. Without that
            // check this used to be a plain timed walk in whatever direction
            // the creature happened to face, which on a short surface is a
            // walk into the wall.
            64..=81 if drop_available => {
                self.dive = true;
                let d = (ctx.rng.range(900, 2000) as u32 * move_scale / 100).max(250);
                self.enter(State::Walk, d);
                true
            }
            82..=99 => self.dash(ctx),
            _ => self.stroll_to(ctx),
        };

        // Nowhere worth going. Rest instead of shuffling: a creature on a
        // surface it cannot cross should look settled, not trapped. This is
        // what produced the pacing at the end of a taskbar and on small
        // windows — every "move" decision walked into a wall and turned round,
        // 130 times in six minutes on a 120px ledge.
        if !moved {
            self.rest(ctx);
        }
    }

    /// Stand, sit, turn on the spot to look around — or show off.
    fn rest(&mut self, ctx: &mut Ctx) {
        let roam = ctx.cfg.roam.min(100) as u32;

        // A trick, now and then, for no reason. Rolled ahead of the ordinary
        // rests rather than as a fourth arm of them, so it keeps its own odds
        // instead of competing with sitting down — and a restless creature
        // shows off more than a placid one.
        if ctx.rng.chance(TRICK_BASE_CHANCE + roam * 12 / 100) {
            self.vx = 0.0;
            self.enter(State::Play, PLAY_MS);
            return;
        }

        let rest_scale = 150 - roam;
        let rng = &mut *ctx.rng;
        let hold = |rng: &mut Rng, lo: i32, hi: i32| {
            (rng.range(lo, hi) as u32 * rest_scale / 100).max(250)
        };
        match rng.below(100) {
            0..=54 => {
                let d = hold(rng, 1400, 4200);
                self.enter(State::Idle, d);
            }
            55..=87 => {
                let d = hold(rng, 2200, 6000);
                self.enter(State::Sit, d);
            }
            _ => {
                self.facing = -self.facing;
                let d = hold(rng, 500, 1200);
                self.enter(State::Idle, d);
            }
        }
    }

    /// A short burst at running speed, for personality.
    ///
    /// Like a stroll, it needs somewhere to go: a dash into a wall is a turn
    /// on the spot with extra steps.
    fn dash(&mut self, ctx: &mut Ctx) -> bool {
        let Some(l) = self.ledge else { return false };
        let (lo, hi) = ((l.x0 + self.margin) as f32, (l.x1 - self.margin) as f32);
        let (room_left, room_right) = (self.x - lo, hi - self.x);
        let go_left = room_left > room_right;
        let room = room_left.max(room_right);
        if room < MIN_TRAVEL * 2.0 {
            return false;
        }
        let speed = (ctx.cfg.speed * self.speed_scale * RUN_MULTIPLIER).max(1.0);
        let d = room.min(speed * 1.4);
        self.facing = if go_left { -1 } else { 1 };
        let ms = (d / speed * 1000.0) as u32;
        self.enter(State::Run, ms.clamp(300, 4_000));
        true
    }

    /// Walk to a chosen spot on the current surface.
    ///
    /// Replaces "walk for 1-3 seconds in a direction decided by a coin flip",
    /// which is a random walk: steps of 55-166px that reverse half the time
    /// return the creature to where it started. Measured on a stock 1920px
    /// taskbar, a quarter of all 30-second stretches covered under 250px and
    /// the worst covered 91px — which reads exactly as pacing on the spot.
    ///
    /// A destination fixes it without any new state, because a walk already
    /// ends when its hold expires: pick a target, set the hold to the time the
    /// journey takes, and the creature arrives. Direction persistence comes out
    /// of committing to somewhere rather than being bolted on.
    fn stroll_to(&mut self, ctx: &mut Ctx) -> bool {
        let Some(l) = self.ledge else { return false };
        let (lo, hi) = ((l.x0 + self.margin) as f32, (l.x1 - self.margin) as f32);
        let span = hi - lo;
        if span < 48.0 {
            return false;
        }

        // Worth the trip: a decent share of the surface, floored so a short
        // ledge still gets crossed and capped so a wide one is not always a
        // full-length march — that is what the sprint is for.
        let min_d = (span * 0.18).clamp(60.0, 220.0);
        let max_d = (span * 0.60).max(min_d + 1.0);

        let (room_left, room_right) = (self.x - lo, hi - self.x);
        let go_left = match (room_left >= min_d, room_right >= min_d) {
            (true, true) => ctx.rng.chance(50),
            (true, false) => true,
            (false, true) => false,
            // Boxed in at both ends: head for the roomier side and take what
            // is there, so it still moves rather than jittering in place.
            (false, false) => room_left > room_right,
        };
        let room = if go_left { room_left } else { room_right };
        if room < MIN_TRAVEL {
            return false;
        }

        let d = if room <= min_d {
            room
        } else {
            let reach = max_d.min(room) - min_d;
            min_d + reach * (ctx.rng.below(1000) as f32 / 1000.0)
        };
        self.facing = if go_left { -1 } else { 1 };
        let speed = (ctx.cfg.speed * self.speed_scale).max(1.0);
        let ms = (d / speed * 1000.0) as u32;
        self.enter(State::Walk, ms.clamp(400, 20_000));
        true
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
        crate::sprites::step_clip(
            &mut self.frame,
            &mut self.frame_acc,
            dt,
            set.clip(a).frame_ms,
            set.frame_count(a),
        );
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
            || matches!(self.state, State::Walk | State::Run | State::Fall | State::Climb)
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
    use crate::platforms::{Monitor, Wall};
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
            vec![Ledge::at(0, 1600, FLOOR_Y)],
        )
    }

    /// A stand-in window handle. Only ever compared, never dereferenced.
    fn fake_window(n: usize) -> HWND {
        n as HWND
    }

    /// A desktop with one window whose title bar the pet can stand on.
    fn world_with_window(win: HWND, x0: i32, x1: i32, y: i32) -> World {
        World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1600, bottom: 1000 },
                work: RECT { left: 0, top: 0, right: 1600, bottom: FLOOR_Y },
            }],
            vec![
                Ledge::at(0, 1600, FLOOR_Y),
                Ledge { x0, x1, y, owner: win },
            ],
        )
    }

    /// Standing on a window's title bar and then moving that window must carry
    /// the creature with it. Before this, the ledge scan was the only thing
    /// that knew a window had moved — and it runs a few times a second, so a
    /// drag slid the window out from under a creature that kept its screen
    /// position: left behind horizontally, and dropped vertically the moment
    /// the window outran the 3px support tolerance.
    #[test]
    fn the_creature_travels_with_the_window_it_stands_on() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = Config::default();
        let win = fake_window(1);
        let (wx0, wy) = (400, 500);

        let mut world = world_with_window(win, wx0, wx0 + 600, wy);
        let mut pet = Pet::new(700.0, wy as f32);
        pet.set_body_width(96, Kind::Pal.body_half_frac());
        let mut rng = Rng::with_seed(3);

        // Land it on the window.
        for _ in 0..8 {
            let mut ctx = Ctx {
                world: &world,
                cursor: (700, wy),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
        }
        assert!(pet.grounded, "should be standing on the window");
        assert_eq!(
            pet.ledge.and_then(|l| l.window()),
            Some(win),
            "should know which window it is on"
        );

        // Drag the window: right and down, faster per tick than the support
        // tolerance, which is the case that used to drop it.
        let (start_x, start_y) = (pet.x, pet.y);
        let (mut wx, mut wyy) = (wx0, wy);
        for _ in 0..10 {
            wx += 17;
            wyy += 9;
            assert!(pet.ride(&world, |h| if std::ptr::eq(h, win) {
                Some((wx, wyy))
            } else {
                None
            }));
            let mut ctx = Ctx {
                world: &world,
                cursor: (700, wy),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            assert!(pet.grounded, "must not fall off a window being dragged");
        }

        // Nine ticks of movement after the first, which only sets the anchor.
        assert_eq!(pet.x - start_x, 17.0 * 9.0, "carried horizontally");
        assert_eq!(pet.y - start_y, 9.0 * 9.0, "carried vertically");

        // And once the window is gone, the ride ends and it falls.
        assert!(!pet.ride(&world, |_| None));
        world = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1600, bottom: 1000 },
                work: RECT { left: 0, top: 0, right: 1600, bottom: FLOOR_Y },
            }],
            vec![Ledge::at(0, 1600, FLOOR_Y)],
        );
        let mut ctx = Ctx {
            world: &world,
            cursor: (700, wy),
            cpu_load: 0.0,
            idle_ms: 0,
            cfg: &cfg,
            rng: &mut rng,
        };
        pet.update(25, &mut ctx, &set);
        assert!(!pet.grounded, "a closed window is not something to stand on");
    }

    /// A window covered by another loses its ledge in the scan. The creature
    /// must not keep riding a title bar it is no longer standing on — that
    /// would leave it hovering over whatever is now in front.
    #[test]
    fn riding_stops_when_the_window_is_covered() {
        let win = fake_window(2);
        let mut pet = Pet::new(700.0, 500.0);
        pet.grounded = true;
        pet.ledge = Some(Ledge { x0: 400, x1: 1000, y: 500, owner: win });

        // The scan still lists it: riding works.
        let with = world_with_window(win, 400, 1000, 500);
        assert!(pet.ride(&with, |_| Some((400, 500))));

        // Covered — the scan no longer offers that window's edge.
        let without = world();
        assert!(
            !pet.ride(&without, |_| Some((400, 500))),
            "a covered window must not be ridden"
        );
    }

    /// Only a creature standing on a window rides. Being carried by the user,
    /// or standing on the taskbar, must not be perturbed by anything a window
    /// does.
    #[test]
    fn nothing_but_a_window_underfoot_is_ridden() {
        let win = fake_window(3);
        let w = world_with_window(win, 400, 1000, 500);

        // On the monitor floor: no owner, no ride.
        let mut floor_pet = Pet::new(700.0, FLOOR_Y as f32);
        floor_pet.grounded = true;
        floor_pet.ledge = Some(Ledge::at(0, 1600, FLOOR_Y));
        assert!(!floor_pet.ride(&w, |_| Some((0, 0))));

        // Held by the user, over a window that is moving.
        let mut held = Pet::new(700.0, 500.0);
        held.grounded = true;
        held.ledge = Some(Ledge { x0: 400, x1: 1000, y: 500, owner: win });
        held.enter(State::Drag, 0);
        assert!(!held.ride(&w, |_| Some((999, 999))));

        // Airborne.
        let mut falling = Pet::new(700.0, 500.0);
        falling.ledge = Some(Ledge { x0: 400, x1: 1000, y: 500, owner: win });
        falling.grounded = false;
        assert!(!falling.ride(&w, |_| Some((999, 999))));
    }

    /// Poking should be worth doing. The reaction is not always the same one,
    /// and the trick needs the ground under it — poking a falling creature is
    /// no reason for it to start dancing in mid-air.
    #[test]
    fn poking_the_creature_sometimes_gets_a_trick() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = Config::default();

        let mut tricks = 0;
        for seed in 0..60u64 {
            let (mut pet, mut rng) = grounded_pet(&set, &cfg);
            let _ = &mut rng;
            let mut r = Rng::with_seed(seed);
            pet.poke(&mut r);
            match pet.state {
                State::Play => tricks += 1,
                State::Alert => {}
                other => panic!("a poke produced {other:?}"),
            }
        }
        // Both outcomes have to actually happen; the exact split is a feel.
        assert!(tricks > 5, "poking never got a trick ({tricks}/60)");
        assert!(tricks < 55, "poking always got a trick ({tricks}/60)");

        // Airborne: the plain reaction, never a trick.
        for seed in 0..40u64 {
            let mut pet = Pet::new(700.0, 300.0);
            pet.grounded = false;
            pet.poke(&mut Rng::with_seed(seed));
            assert_ne!(pet.state, State::Play, "tricks need the ground");
        }

        // A commanded nap outranks a poke, as it does every other reaction.
        let (mut pet, _) = grounded_pet(&set, &cfg);
        pet.force_sleep();
        pet.poke(&mut Rng::with_seed(1));
        assert!(pet.is_sleeping(), "a poke must not end a commanded nap");
    }

    /// A trick runs to the end. Half of one reads as the creature thinking
    /// better of it, and the state is entered from `rest`, which is reached
    /// again the moment the hold expires.
    #[test]
    fn a_trick_is_not_interrupted_half_way() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = Config::default();
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        pet.enter(State::Play, PLAY_MS);
        let mut elapsed = 0u32;
        while elapsed < PLAY_MS - 50 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (800, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            elapsed += 25;
            assert_eq!(pet.state, State::Play, "trick cut short at {elapsed}ms");
        }
        // It stands still while doing it, rather than drifting off the spot.
        assert_eq!(pet.vx, 0.0, "a trick is performed on the spot");
    }

    /// Drop the pet onto the floor and settle it.
    fn grounded_pet(set: &SpriteSet, cfg: &Config) -> (Pet, Rng) {
        let mut pet = Pet::new(800.0, 900.0);
        pet.set_body_width(96, Kind::Pal.body_half_frac());
        let mut rng = Rng::with_seed(7);
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

    /// Floor plus two stacked window ledges, all within hop range.
    fn layered_world() -> World {
        World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1600, bottom: 1000 },
                work: RECT { left: 0, top: 0, right: 1600, bottom: FLOOR_Y },
            }],
            vec![
                Ledge::at(0, 1600, FLOOR_Y),
                Ledge::at(200, 900, 700),
                Ledge::at(300, 1000, 460),
            ],
        )
    }

    /// Drop a pet onto a given world and let it settle.
    fn settle_on(set: &SpriteSet, cfg: &Config, w: &World, x: f32, y: f32) -> (Pet, Rng) {
        settle_on_seeded(set, cfg, w, x, y, 1)
    }

    fn settle_on_seeded(
        set: &SpriteSet, cfg: &Config, w: &World, x: f32, y: f32, seed: u64,
    ) -> (Pet, Rng) {
        let mut pet = Pet::new(x, y);
        pet.set_body_width(96, Kind::Pal.body_half_frac());
        let mut rng = Rng::with_seed(seed);
        for _ in 0..80 {
            let mut ctx = Ctx {
                world: w,
                cursor: (10_000, 10_000),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, set);
            if pet.grounded {
                break;
            }
        }
        assert!(pet.grounded, "pet never landed");
        (pet, rng)
    }

    fn roaming_config(jump: bool) -> Config {
        let mut cfg = Config::default();
        cfg.jump_between_windows = jump;
        cfg.roam = 100; // decide often, so the test does not need to run for ages
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;
        cfg.chase_cursor = false;
        cfg
    }

    /// Every surface the pet stood on during a run.
    fn levels_visited(cfg: &Config, ticks: usize, seed: u64) -> std::collections::BTreeSet<i32> {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let w = layered_world();
        // Start on the middle ledge, so it can go either way.
        let (mut pet, mut rng) = settle_on_seeded(&set, cfg, &w, 550.0, 650.0, seed);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..ticks {
            let mut ctx = Ctx {
                world: &w,
                cursor: (10_000, 10_000),
                cpu_load: 0.0,
                idle_ms: 0,
                cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            if pet.grounded {
                seen.insert(pet.y as i32);
            }
        }
        seen
    }

    /// Trace a leap and report apex, flight time, distance, and where it ended.
    fn trace_leap(from_y: f32, from_x: f32, target: Ledge) -> (f32, f32, f32, i32, i32) {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = roaming_config(true);
        let w = layered_world();
        let (mut pet, mut rng) = settle_on(&set, &cfg, &w, from_x, from_y - 40.0);
        let launch_y = pet.y;
        let launch_x = pet.x;

        let clearance = if target.y as f32 > pet.y {
            HOP_DOWN_CLEARANCE
        } else {
            JUMP_CLEARANCE
        };
        pet.leap_to(target, clearance);

        let (mut apex, mut ms) = (launch_y, 0u32);
        for _ in 0..400 {
            let mut ctx = Ctx { world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                                idle_ms: 0, cfg: &cfg, rng: &mut rng };
            pet.update(16, &mut ctx, &set);
            ms += 16;
            apex = apex.min(pet.y);
            if pet.grounded {
                break;
            }
        }
        (launch_y - apex, ms as f32 / 1000.0, (pet.x - launch_x).abs(),
         pet.y as i32, pet.x as i32)
    }

    /// A leap has to actually arrive. The arc is ballistic and uncorrected, so
    /// if the launch maths is wrong the creature simply misses and falls.
    #[test]
    fn leaps_land_on_their_target() {
        // Up onto the middle ledge from the floor.
        let up = Ledge::at(200, 900, 700);
        let (apex, secs, dist, landed_y, landed_x) = trace_leap(960.0, 550.0, up);
        println!("up   200px: apex {apex:.0}px  {secs:.2}s  {dist:.0}px across  -> y={landed_y} x={landed_x}");
        assert_eq!(landed_y, 700, "should have landed on the target ledge");
        assert!(apex > 260.0, "must clear the target, only rose {apex:.0}px");

        // Down from the top ledge onto the middle one.
        let down = Ledge::at(200, 900, 700);
        let (apex, secs, dist, landed_y, landed_x) = trace_leap(460.0, 550.0, down);
        println!("down 240px: apex {apex:.0}px  {secs:.2}s  {dist:.0}px across  -> y={landed_y} x={landed_x}");
        assert_eq!(landed_y, 700, "should have dropped onto the target ledge");
        assert!(apex < 40.0, "a hop down should barely rise, went up {apex:.0}px");

        // Sideways as well as down: it must travel, not drop straight.
        let far = Ledge::at(200, 900, 700);
        let (_, _, dist, landed_y, _) = trace_leap(460.0, 950.0, far);
        println!("down + across from x=950: {dist:.0}px across -> y={landed_y}");
        assert_eq!(landed_y, 700);
        assert!(dist > 40.0, "barely moved sideways ({dist:.0}px) - dribbled off the edge");
    }

    /// A world shaped like a real desktop: a taskbar, and a big window whose
    /// title bar is far out of jumping reach. Measured from the reporter's
    /// screen — work area bottom 1030, nearest window top 323.
    fn tall_window_world() -> World {
        let mut w = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                work: RECT { left: 0, top: 0, right: 1920, bottom: 1030 },
            }],
            vec![
                Ledge::at(0, 1920, 1030),
                Ledge::at(398, 1806, 323),
            ],
        );
        w.walls = vec![
            Wall { x: 398, y_top: 323, y_bottom: 1030, inward: 1 },
            Wall { x: 1806, y_top: 323, y_bottom: 1030, inward: -1 },
        ];
        w
    }

    /// Regression for "multiple windows on screen but the pet never jumped".
    ///
    /// 1030 - 323 = 707px, far beyond any believable leap, so jumping alone
    /// leaves the creature stranded on the taskbar forever. It has to climb.
    #[test]
    fn climbs_to_a_window_far_above_jumping_reach() {
        assert!(
            707 > JUMP_MAX_RISE,
            "this test is meaningless if the gap is jumpable"
        );
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = roaming_config(true);

        for seed in [1u64, 5, 21, 64] {
            let w = tall_window_world();
            let (mut pet, mut rng) =
                settle_on_seeded(&set, &cfg, &w, 1500.0, 990.0, seed);
            assert_eq!(pet.y as i32, 1030, "should start on the taskbar");

            let mut reached_window = false;
            let mut climbed = false;
            for _ in 0..24_000 {
                let mut ctx = Ctx {
                    world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                };
                pet.update(25, &mut ctx, &set);
                if pet.state == State::Climb {
                    climbed = true;
                    assert_eq!(
                        pet.anim(),
                        Anim::Climb,
                        "climbing must play the climb animation"
                    );
                }
                if pet.grounded && pet.y as i32 == 323 {
                    reached_window = true;
                    break;
                }
            }
            assert!(climbed, "seed {seed}: never even took hold of an edge");
            assert!(reached_window, "seed {seed}: never got onto the window");
        }
    }

    /// The commonest desktop there is: one 1080p screen, one File Explorer
    /// window open in the lower right, and the pet down on the taskbar.
    ///
    /// Worth pinning as its own case because the numbers are so lopsided. The
    /// window's top is 559px up, twice the jump ceiling, so the only way onto it
    /// is the left edge — and that edge is 302px away sideways, most of the
    /// 500px the pet will walk to reach one. Trim either constant and this
    /// perfectly ordinary desktop silently becomes unclimbable.
    fn explorer_window_world() -> World {
        let mut w = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                work: RECT { left: 0, top: 0, right: 1920, bottom: 1035 },
            }],
            vec![
                Ledge::at(0, 1920, 1035),   // taskbar
                Ledge::at(477, 1880, 476),  // the window's title bar
            ],
        );
        w.walls = vec![
            Wall { x: 477, y_top: 476, y_bottom: 1035, inward: 1 },
            Wall { x: 1880, y_top: 476, y_bottom: 1035, inward: -1 },
        ];
        w
    }

    #[test]
    fn climbs_a_single_explorer_window_from_the_taskbar() {
        assert!(1035 - 476 > JUMP_MAX_RISE, "the window top must be out of jumping reach");
        assert!(477 - 175 <= CLIMB_SEEK_DX, "the pet must be willing to walk to the edge");

        let set = sprites::builtin(Kind::Pal, &Palette::default());
        // The stock Roam setting, not the cranked-up one the other roaming
        // tests use: this is about whether an ordinary desktop works, and how
        // long it takes at the default is part of the answer.
        let mut cfg = roaming_config(true);
        cfg.roam = 45;

        for seed in [2u64, 9, 40, 77, 5, 13, 88, 101] {
            let w = explorer_window_world();
            // Bottom left of the desktop, where the pet in the screenshot was.
            let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 175.0, 1000.0, seed);
            assert_eq!(pet.y as i32, 1035, "should start on the taskbar");

            // Budget, not just eventual success. Committing to a chosen edge
            // took these seeds from 37s-4m40s down to 12s-61s; without a bound
            // that improvement can rot back to a random walk and every
            // assertion here would still pass.
            const BUDGET_MS: u32 = 90_000;
            let mut arrived_ms = None;
            for tick in 1..=(BUDGET_MS / 25) {
                let mut ctx = Ctx {
                    world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                };
                pet.update(25, &mut ctx, &set);
                if pet.grounded && pet.y as i32 == 476 {
                    arrived_ms = Some(tick * 25);
                    break;
                }
            }
            assert!(
                arrived_ms.is_some(),
                "seed {seed}: never got up onto the Explorer window in {}s",
                BUDGET_MS / 1000
            );
        }
    }

    /// And it must come back down again rather than living up there.
    #[test]
    fn comes_back_down_from_a_tall_window() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = roaming_config(true);
        let w = tall_window_world();
        // Start already up on the window.
        let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 1000.0, 280.0, 11);
        assert_eq!(pet.y as i32, 323);

        let mut back_down = false;
        for _ in 0..24_000 {
            let mut ctx = Ctx {
                world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                idle_ms: 0, cfg: &cfg, rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            if pet.grounded && pet.y as i32 == 1030 {
                back_down = true;
                break;
            }
        }
        assert!(back_down, "stayed stuck on the window");
    }

    /// Climbing is part of moving between windows, so the toggle governs it.
    #[test]
    fn toggle_off_prevents_climbing() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let cfg = roaming_config(false);
        let w = tall_window_world();
        let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 1500.0, 990.0, 4);
        for _ in 0..20_000 {
            let mut ctx = Ctx {
                world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                idle_ms: 0, cfg: &cfg, rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            assert_ne!(pet.state, State::Climb, "climbed with the toggle off");
            assert_eq!(pet.y as i32, 1030, "left the taskbar with the toggle off");
        }
    }

    /// How long the creature stayed put each time it arrived somewhere, in ms.
    fn dwell_times(cfg: &Config, seed: u64) -> Vec<u32> {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let w = layered_world();
        let (mut pet, mut rng) = settle_on_seeded(&set, cfg, &w, 550.0, 650.0, seed);

        let mut dwells = Vec::new();
        let (mut on_ground, mut since) = (true, 0u32);
        for _ in 0..24_000 {
            let mut ctx = Ctx {
                world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                idle_ms: 0, cfg, rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            match (on_ground, pet.grounded) {
                (true, true) => since += 25,
                (true, false) => {
                    dwells.push(since);
                    on_ground = false;
                }
                (false, true) => {
                    on_ground = true;
                    since = 0;
                }
                (false, false) => {}
            }
        }
        dwells
    }

    /// Reported as "after jumping onto a new platform it immediately jumps
    /// down". Landing used to be followed by a 700ms idle and then a free
    /// decision, and the direction bias favours dropping when high up — so a
    /// climb ended in an instant hop back and the creature yo-yoed.
    #[test]
    fn explores_a_new_surface_before_moving_on() {
        // roam 100 is the most impatient setting, so the shortest settle.
        let cfg = roaming_config(true);
        let floor = (SETTLE_BASE_MS * (140 - 100) / 100).max(SETTLE_MIN_MS);

        for seed in [1u64, 8, 23, 55] {
            let dwells = dwell_times(&cfg, seed);
            assert!(
                dwells.len() >= 3,
                "seed {seed}: barely moved, only {} departures",
                dwells.len()
            );
            let shortest = *dwells.iter().min().unwrap();
            assert!(
                shortest + 25 >= floor,
                "seed {seed}: left after only {shortest}ms, expected >= {floor}ms                  (all dwells: {dwells:?})"
            );
        }
    }

    /// It used to only ever climb — `ledge_above` was the only target, so the
    /// pet accumulated on the topmost window and stayed there.
    #[test]
    fn hops_both_up_and_down() {
        // Several seeds: the behaviour is randomised, and a property that only
        // holds for one lucky seed is not a property.
        for seed in [1u64, 7, 13, 42, 99] {
            let seen = levels_visited(&roaming_config(true), 16_000, seed);
            assert!(
                seen.contains(&460),
                "seed {seed}: never reached the top ledge; visited {seen:?}"
            );
            assert!(
                seen.contains(&FLOOR_Y),
                "seed {seed}: never came back down; visited {seen:?}"
            );
        }
    }

    /// With the toggle off it stays put on whatever it is standing on.
    #[test]
    fn toggle_off_keeps_it_on_one_surface() {
        let seen = levels_visited(&roaming_config(false), 12_000, 3);
        assert_eq!(
            seen.len(),
            1,
            "should never change surface with jumping off; visited {seen:?}"
        );
        assert!(seen.contains(&700), "should have stayed on the middle ledge");
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

    /// A commanded nap outranks everything except "Wake up". Carrying the pet
    /// somewhere else, an app opening, a reminder firing, a poke, a busy CPU —
    /// none of them may rouse it.
    #[test]
    fn nothing_but_wake_up_ends_a_commanded_sleep() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = chase_config();
        cfg.cpu_annoy_percent = 50;
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        pet.force_sleep();
        assert!(pet.is_sleeping());

        // Carried across the desktop and put down again.
        pet.begin_drag();
        pet.drag_to(500.0, 300.0);
        pet.end_drag();
        assert!(pet.is_sleeping(), "being carried must not cancel the nap");

        // An app opening, a reminder and a poke all come through `notice`.
        pet.notice(Some(900));
        pet.notice(None);
        assert_ne!(pet.state, State::Alert, "notice woke a commanded sleep");

        // Land, then run with the machine pegged and the cursor jumping about.
        for i in 0..800 {
            let mut ctx = Ctx {
                world: &w,
                cursor: (100 + (i % 700), FLOOR_Y - 200),
                cpu_load: 0.95, // well over the annoy threshold
                idle_ms: 0,     // input every tick
                cfg: &cfg,
                rng: &mut rng,
            };
            pet.update(25, &mut ctx, &set);
            if i % 97 == 0 {
                pet.notice(None);
            }
        }
        assert_eq!(pet.state, State::Sleep, "commanded sleep must hold");

        pet.wake();
        assert!(!pet.is_sleeping(), "'Wake up' is the one thing that works");
    }

    /// The automatic nap is the opposite: it exists to get out of the way, so
    /// the first sign of life still ends it.
    #[test]
    fn idle_sleep_still_wakes_on_input() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = chase_config();
        cfg.sleep_after_idle_secs = 5;
        let (mut pet, mut rng) = grounded_pet(&set, &cfg);
        let w = world();

        let tick = |pet: &mut Pet, rng: &mut Rng, idle_ms: u64| {
            let mut ctx = Ctx {
                world: &w,
                cursor: (400, FLOOR_Y),
                cpu_load: 0.0,
                idle_ms,
                cfg: &cfg,
                rng,
            };
            pet.update(25, &mut ctx, &set);
        };

        for _ in 0..400 {
            tick(&mut pet, &mut rng, 30_000);
        }
        assert_eq!(pet.state, State::Sleep, "should have nodded off on its own");
        assert!(!pet.forced_sleep, "the idle timer is not a command");

        tick(&mut pet, &mut rng, 0);
        assert_ne!(pet.state, State::Sleep, "input should end an idle nap");
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

    /// A walk must get somewhere.
    ///
    /// It used to be "go this way for 1-3 seconds", with the direction
    /// re-flipped by a coin toss each time — a random walk, which returns to
    /// where it started. Measured on a stock 1920px taskbar: **41% of all
    /// movements travelled under 100px** and the median was 125px, which is
    /// the shuffling-on-the-spot people notice. With a destination the median
    /// is 652px and 15% are short, those being the ones that run out of ledge.
    ///
    /// Measured per movement rather than over a time window. A window metric
    /// cannot tell pacing from resting, and a resting creature is the roam
    /// dial working, not a bug — an earlier version of this test measured
    /// windows and passed happily with the fix removed.
    #[test]
    fn a_walk_covers_real_ground() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = Config::default(); // stock settings, as shipped
        cfg.chase_cursor = false;
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;

        // One 1920-wide taskbar and nothing else, which is most desktops.
        let w = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                work: RECT { left: 0, top: 0, right: 1920, bottom: 1035 },
            }],
            vec![Ledge::at(0, 1920, 1035)],
        );

        let mut moves: Vec<f32> = Vec::new();
        for seed in [1u64, 2, 3, 17, 44, 61, 90] {
            let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 960.0, 1000.0, seed);
            let mut prev = pet.state;
            let mut from = pet.x;
            for _ in 0..12_000 {
                let mut ctx = Ctx {
                    world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                };
                pet.update(25, &mut ctx, &set);
                let moving = matches!(pet.state, State::Walk | State::Run);
                let was = matches!(prev, State::Walk | State::Run);
                if moving && !was {
                    from = pet.x;
                } else if was && (!moving || pet.state != prev) {
                    moves.push((pet.x - from).abs());
                    from = pet.x;
                }
                prev = pet.state;
            }
        }

        assert!(moves.len() > 50, "only {} movements to judge", moves.len());
        moves.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = moves[moves.len() / 2];
        let short = moves.iter().filter(|&&d| d < 100.0).count() as f32 / moves.len() as f32;

        assert!(
            median >= 300.0,
            "median movement is only {median:.0}px on a 1920px taskbar"
        );
        assert!(
            short < 0.25,
            "{:.0}% of movements travel under 100px — the creature is pacing",
            short * 100.0
        );
    }

    /// How far does one walk actually get? `cargo test -- --ignored --nocapture pacing_metric`.
    #[test]
    #[ignore]
    fn pacing_metric() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = Config::default();
        cfg.chase_cursor = false;
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;
        let w = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                work: RECT { left: 0, top: 0, right: 1920, bottom: 1035 },
            }],
            vec![Ledge::at(0, 1920, 1035)],
        );
        let mut all: Vec<f32> = Vec::new();
        let mut sprints = 0;
        for seed in [1u64, 2, 3, 17, 44, 61, 90] {
            let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 960.0, 1000.0, seed);
            let mut prev = pet.state;
            let mut seg_start = pet.x;
            for _ in 0..12_000 {
                let mut ctx = Ctx {
                    world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                };
                pet.update(25, &mut ctx, &set);
                let now = pet.state;
                let was_move = matches!(prev, State::Walk | State::Run);
                let is_move = matches!(now, State::Walk | State::Run);
                if !was_move && is_move {
                    seg_start = pet.x;
                } else if was_move && !is_move {
                    all.push((pet.x - seg_start).abs());
                } else if was_move && is_move && now != prev {
                    all.push((pet.x - seg_start).abs());
                    seg_start = pet.x;
                }
                if now == State::Run && prev != State::Run {
                    sprints += 1;
                }
                prev = now;
            }
        }
        all.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |q: f32| all[((all.len() as f32 - 1.0) * q) as usize];
        let short = all.iter().filter(|&&d| d < 100.0).count() as f32 / all.len() as f32 * 100.0;
        println!(
            "{} moves over 35 minutes: median {:.0}px, 10th {:.0}px, 90th {:.0}px;              {short:.0}% travel under 100px; {sprints} runs started",
            all.len(),
            pct(0.5),
            pct(0.1),
            pct(0.9),
        );
    }

    /// A creature on a surface it cannot cross must settle, not pace.
    ///
    /// Reported as "gets caught in a loop near the end of the screen, walking
    /// back and forth a hundred times before going anywhere". The cause was
    /// that a "move" decision always moved: on a short ledge every one of them
    /// was two steps into a wall and a turn around. Measured on a 120px window
    /// ledge with hopping off, the creature turned **130 times in six
    /// minutes**. It now rests when there is nowhere worth walking to, and
    /// treats anything under `CRAMPED_SPAN` of usable width as a perch to sit
    /// on rather than a place to roam.
    #[test]
    fn a_narrow_perch_is_for_sitting_on() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());

        for width in [96i32, 120, 180, 260] {
            for jump in [true, false] {
                let mut cfg = Config::default();
                cfg.chase_cursor = false;
                cfg.jump_between_windows = jump;
                cfg.sleep_after_idle_secs = 86_400;
                cfg.cpu_annoy_percent = 100;
                let w = World::for_test(
                    vec![Monitor {
                        rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                        work: RECT { left: 0, top: 0, right: 1920, bottom: 1035 },
                    }],
                    vec![
                        Ledge::at(0, 1920, 1035),
                        Ledge::at(40, 40 + width, 880),
                    ],
                );

                let mut turns = 0;
                let mut ticks_on_perch = 0;
                for seed in 1..=6u64 {
                    let (mut pet, mut rng) =
                        settle_on_seeded(&set, &cfg, &w, (40 + width / 2) as f32, 860.0, seed);
                    let mut facing = pet.facing;
                    for _ in 0..2_400 {
                        // one minute each
                        let mut ctx = Ctx {
                            world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                            idle_ms: 0, cfg: &cfg, rng: &mut rng,
                        };
                        pet.update(25, &mut ctx, &set);
                        if pet.y as i32 == 880 {
                            ticks_on_perch += 1;
                            if pet.facing != facing {
                                turns += 1;
                            }
                        }
                        facing = pet.facing;
                    }
                }

                // Per minute actually spent up there, so a creature that hops
                // away quickly is not credited for good behaviour.
                let minutes = (ticks_on_perch as f32 * 0.025 / 60.0).max(0.05);
                let per_min = turns as f32 / minutes;
                assert!(
                    per_min <= 8.0,
                    "{width}px perch, jumping {}: turned around {per_min:.0} times a minute",
                    if jump { "on" } else { "off" },
                );
            }
        }
    }

    /// A maximised window must not pin the creature to the screen edge.
    ///
    /// Reported as pacing back and forth near the corner "like 100 times",
    /// and only when something was running fullscreen. That last detail is the
    /// whole diagnosis: a maximised window puts its climbable edges exactly on
    /// the screen edges, and the creature stands half a body width from the end
    /// of a surface, so it can never get the 14px from one that taking hold
    /// requires. It walked at the edge, was clamped by `confine_to_ledge`,
    /// turned around, and aimed at the same edge on the very next decision —
    /// 136 turns in three minutes, 133 of 180 seconds spent within 150px of the
    /// screen edge.
    ///
    /// Offering an unreachable edge is worse than offering none, so the world
    /// no longer offers one.
    #[test]
    fn a_fullscreen_window_does_not_trap_the_creature() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = Config::default();
        cfg.chase_cursor = false;
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;

        let mut w = World::for_test(
            vec![Monitor {
                rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
                work: RECT { left: 0, top: 0, right: 1920, bottom: 1035 },
            }],
            vec![
                Ledge::at(0, 1920, 1035),
                Ledge::at(0, 1920, 0),
            ],
        );
        w.walls = vec![
            Wall { x: 0, y_top: 0, y_bottom: 1035, inward: 1 },
            Wall { x: 1920, y_top: 0, y_bottom: 1035, inward: -1 },
        ];

        for seed in [1u64, 2, 3, 8, 21] {
            let (mut pet, mut rng) = settle_on_seeded(&set, &cfg, &w, 700.0, 1000.0, seed);
            let mut turns = 0;
            let mut facing = pet.facing;
            let mut pinned = 0;
            for _ in 0..7_200 {
                // three minutes
                let mut ctx = Ctx {
                    world: &w, cursor: (10_000, 10_000), cpu_load: 0.0,
                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                };
                pet.update(25, &mut ctx, &set);
                if pet.facing != facing {
                    turns += 1;
                    facing = pet.facing;
                }
                if pet.x < 150.0 || pet.x > 1770.0 {
                    pinned += 1;
                }
            }
            assert!(
                turns <= 30,
                "seed {seed}: turned around {turns} times in three minutes"
            );
            let pinned_s = pinned as f32 * 0.025;
            assert!(
                pinned_s <= 90.0,
                "seed {seed}: spent {pinned_s:.0}s of 180 stuck against a screen edge"
            );
        }
    }

    /// The creature must not get pinned at a screen edge on *any* setup.
    ///
    /// Every previous version of this bug was a different arrangement of the
    /// same trap: something the creature aims at but cannot reach, re-chosen on
    /// every decision, with `confine_to_ledge` turning it round in between. Two
    /// were found only because the user said which setup they were on. So this
    /// stops enumerating setups and generates them: screen size, taskbar
    /// height, monitor offset (including negative, for a screen to the left of
    /// the primary), how far a big window is inset from the edges, and how big
    /// the sprite is — a 4x monkey on a 4K screen keeps its centre 54px from a
    /// ledge end, which is what made 14px-off-the-edge unreachable.
    ///
    /// The inset sweep matters most: at 0 the window edges land exactly on the
    /// screen edges, and around a body's half width they land just inside it,
    /// which is where the 2560x1440 report came from.
    #[test]
    fn no_setup_pins_the_creature_to_an_edge() {
        let set = sprites::builtin(Kind::Pal, &Palette::default());
        let mut cfg = Config::default();
        cfg.chase_cursor = false;
        cfg.sleep_after_idle_secs = 86_400;
        cfg.cpu_annoy_percent = 100;

        let mut worst = (0u32, String::new());
        for &(sw, sh) in &[(1366i32, 768i32), (1920, 1080), (2560, 1440), (3840, 2160)] {
            for &origin in &[0i32, -2560, 1920] {
                for &taskbar in &[40i32, 48, 72] {
                    for &inset in &[0i32, 8, 24, 40, 60, 120] {
                        for &body in &[64i32, 96, 144] {
                            let mon = RECT {
                                left: origin,
                                top: 0,
                                right: origin + sw,
                                bottom: sh,
                            };
                            let work = RECT { bottom: sh - taskbar, ..mon };
                            let floor = work.bottom;
                            let (wl, wr) = (origin + inset, origin + sw - inset);
                            let mut w = World::for_test(
                                vec![Monitor { rect: mon, work }],
                                vec![
                                    Ledge::at(origin, origin + sw, floor),
                                    Ledge::at(wl, wr, inset.min(60)),
                                ],
                            );
                            w.walls = vec![
                                Wall { x: wl, y_top: inset.min(60), y_bottom: floor, inward: 1 },
                                Wall { x: wr, y_top: inset.min(60), y_bottom: floor, inward: -1 },
                            ];

                            let (mut pet, mut rng) = settle_on_seeded(
                                &set, &cfg, &w,
                                (origin + sw / 2) as f32, (floor - 40) as f32, 3,
                            );
                            pet.set_body_width(body, 0.375);

                            let mut turns = 0u32;
                            let mut facing = pet.facing;
                            for _ in 0..4_800 {
                                // two minutes
                                let mut ctx = Ctx {
                                    world: &w, cursor: (1_000_000, 1_000_000), cpu_load: 0.0,
                                    idle_ms: 0, cfg: &cfg, rng: &mut rng,
                                };
                                pet.update(25, &mut ctx, &set);
                                if pet.facing != facing {
                                    turns += 1;
                                    facing = pet.facing;
                                }
                            }
                            if turns > worst.0 {
                                worst = (
                                    turns,
                                    format!(
                                        "{sw}x{sh} at x={origin}, taskbar {taskbar}, window inset {inset}, body {body}"
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        // Two minutes of ordinary roaming turns the creature around a handful
        // of times. Anything approaching a turn a second is the trap.
        assert!(
            worst.0 <= 40,
            "turned around {} times in two minutes on {}",
            worst.0,
            worst.1
        );
    }
}
