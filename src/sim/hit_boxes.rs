//! Hit boxes: every active phase with a hit box hurts the targets inside it, in the
//! collision step, before orbs.
//!
//! - A hit box follows its attacker, or stays on the point an on-hit rune's attack fired at,
//!   and faces the way the attack faced when it started. Its
//!   offset is forward along that facing and to the attacker's right as seen from above: a
//!   quarter turn clockwise on the screen, so facing down the screen its right is the screen's
//!   left.
//! - A hit box is anchored at its attacker, or at the target's spot (where the target stood when
//!   the attack began); a line is a rectangle from its start along the facing turned by its
//!   angle. Zones (`zones`) are tested the same way, after the hit boxes, each target once.
//! - A circle hits a target whose circle overlaps it; an arc, a target whose circle overlaps
//!   the sector `half_angle` to each side of the facing. Integer only: squared distances in 64
//!   or 128 bits, and the arc's edges and angle from the sine table.
//! - Friendly hit boxes hit the boss, its parts and the enemies (found through the enemy grid),
//!   in entity-ID order; enemy hit boxes hit the present players in slot order, then the companions
//!   once. A hit box
//!   hits every target inside it (cleave), each once per active phase, or again `rehit_ticks`
//!   after its last hit. A homing missile's hits only the first, once, and the
//!   missile is gone once every hit box has landed. A target that cannot be hurt (invulnerable, dead) is not hit and not
//!   counted as hit.
//! - The orb clear: on the first active tick of a player's full-gauge attack, every breakable orb
//!   within the run rules' `orb_clear_radius` of that player is removed before the swings land
//!   (`clear_orbs`), reported as a broken orb, with no burst.
//! - Attackers resolve in entity-ID order, and every hit box active at the start of the step
//!   lands, even if its attacker dies earlier in the step: hits on the same tick trade.

mod hit_sides;

use super::{World, aided, attacks, hitter, knock_enemy, player, pull_enemy, stun};
use crate::attack::{Anchor, DamageType, HitBox, Offset, PhaseKind, ZoneShape};
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rune::Rune;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Dir, Smash, State};
use crate::trig;
use hit_sides::{hit_companions_side, hit_enemy_side, hit_players_side, hits_of};

use super::entity::Who;
use super::grid::Grid;
use super::scratch::{RunEnd, Scratch};
use super::{Blame, Shot, enemy_grid, hurt_player, overlaps};

/// What a swing is made by, and so where the targets it has hit are kept.
#[derive(Clone, Copy)]
enum Source {
    /// The attack run at this place in `State::runs`.
    Run(usize),
    /// The zone at this place in `State::zones` (core Z1).
    Zone(usize),
}

/// One hit box or zone that is active this tick.
pub(super) struct Swing {
    source: Source,
    owner: EntityId,
    slot: u8,
    friendly: bool,
    area: Area,
    damage: u32,
    damage_type: Option<DamageType>,
    rehit_ticks: Option<u32>,
    breaks_charge: bool,
    smash: Option<Smash>,
    combo: bool,
    /// The poise damage the hit phase names, if any (`AttackPhase::poise_damage`).
    poise_damage: Option<u32>,
    /// The stagger the hit phase names, if any (`AttackPhase::stagger`).
    stagger: Option<u32>,
    rune: Option<Rune>,
    /// A homing missile's: one hit, then nothing.
    once: bool,
}

impl Swing {
    /// The place of its source in `State::runs` or `State::zones`.
    fn place(&self) -> usize {
        match self.source {
            Source::Run(i) | Source::Zone(i) => i,
        }
    }
}

/// Puts the tick's hit boxes in owner, then slot order, in place: the stable sort allocates a
/// scratch buffer the size of the crowd's swings, and a steady tick allocates nothing (T1b). The
/// place is the tie-break, so the order is total. It equals the stable sort's for `active_swings`'
/// output, which lists the runs in `State::runs` order, so tied swings come in ascending place
/// (ADR-SIM-3); a caller that lists them in another order gets place order instead. Run before
/// `zone_swings`: a run's and a zone's place share one index space, so every swing here is a run.
fn sort_swings(swings: &mut [Swing]) {
    debug_assert!(swings.iter().all(|w| matches!(w.source, Source::Run(_))));
    swings.sort_unstable_by_key(|w| (w.owner.0, w.slot, w.place()));
}

/// The ground a hit box covers: a circle at `centre`, narrowed to a sector when `arc` is set.
pub(super) struct Area {
    centre: Point,
    facing: Dir,
    /// The circle's radius; a line's reach from its start: its length and half its width, which
    /// holds every point of it.
    radius: i32,
    /// `None` for a circle, or an arc of 180 degrees or more to each side, which is one.
    arc: Option<Sector>,
    /// A rectangle instead of a circle or an arc.
    line: Option<Line>,
}

/// A rectangle from `Area::centre` along `dir`.
struct Line {
    dir: Dir,
    length: i32,
    /// Half the width: the reach to each side.
    half_width: i32,
}

impl Line {
    fn new(dir: Dir, length: Fx, width: Fx) -> Line {
        Line {
            dir,
            length: length.0,
            half_width: width.0 / 2,
        }
    }

    /// The area of a line starting at `centre`.
    fn area(self, centre: Point) -> Area {
        Area {
            centre,
            facing: self.dir,
            radius: self.length.saturating_add(self.half_width),
            arc: None,
            line: Some(self),
        }
    }

    /// Whether the circle at `d` (from the line's start) with `radius` touches the rectangle:
    /// the nearest point of it to the circle's centre is within `radius`. Squared and scaled by
    /// `trig::SCALE`, so no root is taken.
    fn touches(&self, d: (i128, i128), radius: i32) -> bool {
        let (ex, ey) = (i128::from(self.dir.x), i128::from(self.dir.y));
        let scale = i128::from(trig::SCALE);
        let along = d.0 * ex + d.1 * ey;
        let across = d.0 * ey - d.1 * ex;
        let reach = i128::from(self.half_width) * scale;
        let past_along = along - along.clamp(0, i128::from(self.length) * scale);
        let past_across = across - across.clamp(-reach, reach);
        let r = i128::from(radius) * scale;
        past_along * past_along + past_across * past_across <= r * r
    }
}

/// What an arc's test needs, worked out once per swing rather than once per target: the cosine
/// of its half angle and its two edges.
struct Sector {
    cos: i32,
    edges: [Dir; 2],
}

impl Sector {
    fn new(facing: Dir, half_angle: i32) -> Option<Sector> {
        (half_angle < 180 * trig::PER_DEGREE).then(|| Sector {
            cos: trig::sin_cos(half_angle).1,
            edges: [half_angle, -half_angle].map(|turn| super::attacks::rotate(facing, turn)),
        })
    }
}

/// Every active hit box hurts what it covers. `grid` is the enemy grid, built here if a
/// friendly hit box needs it and left for the orbs after.
// Kept out of `sim::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-SIM-5).
#[inline(never)]
pub(super) fn resolve(
    stage: &World,
    s: &mut State,
    grid: &mut Option<Grid>,
    events: &mut Vec<DomainEvent>,
) {
    let mut swings = std::mem::take(&mut s.scratch.swings);
    let mut found = std::mem::take(&mut s.scratch.found);
    let mut bits = std::mem::take(&mut s.scratch.found_bits);
    active_swings(stage, s, &mut swings);
    sort_swings(&mut swings);
    // The zones land after the hit boxes, in the order they were placed.
    zone_swings(s, &mut swings);
    if !s.orbs.is_empty() {
        for me in s.players.indices() {
            break_orbs(stage, s, me, &swings, events);
        }
        if stage.has_gauge && stage.tuning.orb_clear_radius > 0 {
            clear_orbs(stage, s, events);
        }
    }
    land(stage, s, grid, &swings, (&mut bits, &mut found), events);
    if swings.iter().any(|w| w.once) {
        super::attacks::retain_runs(s, |r| r.homing.is_none() || r.hits.is_empty());
        s.move_index.rebuild(stage, &s.runs);
    }
    s.scratch.swings = swings;
    s.scratch.found = found;
    s.scratch.found_bits = bits;
}

/// The cage beams, then every swing, one after another over the runs the pass collected them
/// from.
fn land(
    stage: &World,
    s: &mut State,
    grid: &mut Option<Grid>,
    swings: &[Swing],
    (bits, found): (&mut Vec<u64>, &mut Vec<usize>),
    events: &mut Vec<DomainEvent>,
) {
    // A run that ends in the pass (a player down, a hold broken, a stun) is ended after the
    // last swing, so the places the swings hold stay valid (ADR-SIM-7).
    s.scratch.ends_pinned = Some(s.runs.len());
    if s.next_orb_group > 0 {
        // A beam's `last_hit` is shared, so a beam hurts at most one player per `rehit_ticks`,
        // the lowest slot first.
        for me in s.players.indices() {
            cage_beams(stage, s, me, events);
        }
    }
    let (gauge, tempo) = (stage.has_gauge, stage.has_tempo);
    for w in swings {
        if w.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, w, (bits, found), events);
            if (gauge || tempo) && w.combo {
                // The enemies this swing hit now, each counted once: the weapon gauge's fill and
                // the combo tempo's stack.
                let tick = s.tick;
                let n = u32::try_from(hits_of(s, w).iter().filter(|h| h.1 == tick).count())
                    .unwrap_or(u32::MAX);
                if gauge {
                    super::player::gauge_from_hits(stage, s, w.owner, n, events);
                }
                if tempo {
                    super::player::tempo_from_hits(stage, s, w.owner, n, events);
                }
            }
        } else {
            hit_players_side(stage, s, w, events);
            hit_companions_side(stage, s, w, events);
        }
    }
    apply_ends(stage, s);
}

/// Ends the runs that ended during the pass, in the order they were asked for, among the runs
/// that existed when it began: a run a hit started in the pass is not touched.
pub(super) fn apply_ends(stage: &World, s: &mut State) {
    let pinned = s.scratch.ends_pinned.take().unwrap_or(s.runs.len());
    if s.scratch.ends_pending.is_empty() {
        return;
    }
    let ends = std::mem::take(&mut s.scratch.ends_pending);
    let (tick, mut at) = (s.tick, 0);
    s.runs.retain_mut(|r| {
        let place = at;
        at += 1;
        let ended = place < pinned
            && ends.iter().any(|e| match *e {
                RunEnd::Slot(owner, slot) => r.owner == owner && r.slot == slot,
                RunEnd::All(owner) | RunEnd::Cancel(owner) => r.owner == owner,
            });
        if ended {
            // A cancel cools a channelled run's token, once however many ends asked.
            let cancelled = ends.contains(&RunEnd::Cancel(r.owner));
            if let Some(pool) = cancelled
                .then(|| super::attacks::cancelled_token(stage, r))
                .flatten()
            {
                super::attacks::cool_into(&mut s.ranged_cooling, tick, stage, pool);
            }
            Scratch::recycle_hits(&mut s.scratch.hit_lists, std::mem::take(&mut r.hits));
        }
        !ended
    });
    s.move_index.rebuild(stage, &s.runs);
    let mut ends = ends;
    ends.clear();
    s.scratch.ends_pending = ends;
}

/// A breakable enemy orb that a hit box of the main character covers is removed, with no burst,
/// and reported once (core Z2, C24). It is gone before the orbs land, so it hurts nobody this
/// tick.
fn break_orbs(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    swings: &[Swing],
    events: &mut Vec<DomainEvent>,
) {
    let owner = s.players.id(me);
    let mine = |w: &&Swing| w.friendly && w.owner == owner;
    s.orbs.retain(|b| {
        if !b.breakable {
            return true;
        }
        let hit = swings
            .iter()
            .filter(mine)
            .any(|w| covers(w, b.at, stage.tuning.orb_radius));
        if hit {
            events.push(DomainEvent::OrbBroken {
                owner: b.owner,
                at: b.at,
                player: Some(me),
            });
        }
        !hit
    });
}

/// The full-gauge smash's clear: on the first active tick of a player's full-gauge attack every
/// breakable orb within the run rules' `orb_clear_radius` of that player's centre (edge included)
/// is removed, with the same event as a broken orb and no burst. The attack is told by its
/// compiled index, which the pack check keeps apart from every attack of the base tree. One
/// player's clear is the party's. Zones and unbreakable orbs stay.
fn clear_orbs(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let radius = i64::from(stage.tuning.orb_clear_radius);
    for me in s.players.indices() {
        let Some(crate::expertise::Gauge {
            spend: crate::expertise::GaugeSpend::FullSmash { attack },
            ..
        }) = stage.gauge_of(s, me)
        else {
            continue;
        };
        let (owner, attack) = (s.players.id(me), stage.attack_index(attack, None));
        let starting = s.runs.iter().any(|r| {
            r.owner == owner
                && r.slot == super::player::COMBO_SLOT
                && r.attack == attack
                && r.tick == 0
                && first_active(stage, r)
        });
        if !starting {
            continue;
        }
        let from = s.players[me].at;
        s.orbs.retain(|b| {
            let (dx, dy) = (
                i64::from(b.at.x.0) - i64::from(from.x.0),
                i64::from(b.at.y.0) - i64::from(from.y.0),
            );
            let cleared = b.breakable && dx * dx + dy * dy <= radius * radius;
            if cleared {
                events.push(DomainEvent::OrbBroken {
                    owner: b.owner,
                    at: b.at,
                    player: Some(me),
                });
            }
            !cleared
        });
    }
}

/// Whether `r` is in the first active phase of its attack (the one that clears its hit list).
fn first_active(stage: &World, r: &super::state::AttackRun) -> bool {
    let kind = |i: usize| super::attacks::phase(stage, r, i).kind;
    kind(r.phase as usize) == PhaseKind::Active
        && (r.phase == 0 || kind(r.phase as usize - 1) != PhaseKind::Active)
}

/// The beams of the orbs' cages hurt the main character (Z3a-3), a beam at most once per its
/// `rehit_ticks`; an orb broken just above is gone, and its beams with it. Each beam is a line
/// from one orb to its neighbour, tested like a hit box line; its last hit is kept on the orb it
/// starts from.
fn cage_beams(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    for i in 0..s.orbs.len() {
        let p = &s.players[me];
        // An invulnerable player is only looked at while a perfect-evade window is open: the tick
        // cost of every other evade stays what it was.
        if p.hp == 0 || (p.invulnerable > 0 && p.combat.perfect_window == 0) {
            return;
        }
        let Some(j) = super::beam_to(&s.orbs, i) else {
            continue;
        };
        let (from, to) = (&s.orbs[i], &s.orbs[j]);
        let from_program = from.program;
        let Some(link) = from.link else {
            continue;
        };
        if link.last_hit.is_some_and(|t| s.tick - t < link.rehit_ticks) {
            continue;
        }
        let dx = i64::from(to.at.x.0) - i64::from(from.at.x.0);
        let dy = i64::from(to.at.y.0) - i64::from(from.at.y.0);
        let length = i32::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i32::MAX);
        let line = Line::new(super::attacks::normalize(dx, dy), Fx(length), link.width);
        let d = (
            i128::from(p.at.x.0) - i128::from(from.at.x.0),
            i128::from(p.at.y.0) - i128::from(from.at.y.0),
        );
        if !line.touches(d, stage.tuning.player_radius) {
            continue;
        }
        if p.invulnerable > 0 {
            // The beam is stopped by the invulnerability: a near miss, or nothing at all.
            if p.combat.perfect_window > 0 {
                super::player::near_miss(stage, s, me, events);
            }
            return;
        }
        let shot = Shot {
            at: p.at,
            owner: from.owner,
            friendly: false,
            damage: link.damage,
            damage_type: from.damage_type,
            breaks_charge: from.breaks_charge,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
            dot: false,
        };
        if let Some(l) = s.orbs[i].link.as_mut() {
            l.last_hit = Some(s.tick);
        }
        let by = Blame::of_program(HitKind::Link, from_program);
        hurt_player(stage, s, me, super::aided(stage, s, shot), by, events);
    }
}

fn active_swings(stage: &World, s: &State, swings: &mut Vec<Swing>) {
    swings.clear();
    for (i, r) in s.runs.iter().enumerate() {
        let p = super::attacks::phase(stage, r, r.phase as usize);
        let (PhaseKind::Active, Some(h)) = (p.kind, p.hit_box()) else {
            continue;
        };
        let Some(owner) = s.find(r.owner) else {
            continue;
        };
        let from = origin_of(s, r, h, owner);
        let area = area_of(h, from, r.facing);
        swings.push(Swing {
            source: Source::Run(i),
            owner: r.owner,
            slot: r.slot,
            friendly: r.friendly,
            area,
            damage: super::attacks::phase_damage(stage, r),
            damage_type: p.damage_type,
            rehit_ticks: p.rehit_ticks,
            breaks_charge: p.breaks_charge,
            smash: r.smash,
            combo: super::attacks::is_combo(s, r),
            poise_damage: p.poise_damage,
            stagger: p.stagger,
            rune: super::runes::of_run(r),
            once: r.homing.is_some(),
        });
    }
}

/// The point run `r`'s hit box `h` is centred from: the target's `spot` for a target-anchored
/// box that has one, else the run's own `at`, else `owner`'s position.
#[inline(always)]
pub(super) fn origin_of(s: &State, r: &super::state::AttackRun, h: &HitBox, owner: Who) -> Point {
    match (h.anchor(), r.spot) {
        (Anchor::Target, Some(spot)) => spot,
        _ => r.at.unwrap_or_else(|| s.at(owner)),
    }
}

/// The area of hit box `h` centred from `from` and facing `facing`.
#[inline(always)]
pub(super) fn area_of(h: &HitBox, from: Point, facing: Dir) -> Area {
    let at_offset = |offset: &Offset| centre(from, facing, offset.forward.0, offset.right.0);
    match h {
        HitBox::Circle { offset, radius, .. } => circle(at_offset(offset), facing, radius.0, None),
        HitBox::Arc {
            offset,
            radius,
            half_angle,
            ..
        } => circle(
            at_offset(offset),
            facing,
            radius.0,
            Sector::new(facing, *half_angle),
        ),
        HitBox::Line {
            offset,
            angle,
            length,
            width,
            ..
        } => Line::new(super::attacks::rotate(facing, *angle), *length, *width)
            .area(at_offset(offset)),
    }
}

/// Adds a swing for each zone that is active this tick.
fn zone_swings(s: &State, swings: &mut Vec<Swing>) {
    for (i, z) in s.zones.iter().enumerate() {
        if z.wind_left > 0 || z.warn_only {
            continue;
        }
        let area = match &z.shape {
            ZoneShape::Circle { radius } => circle(z.at, z.dir, radius.0, None),
            ZoneShape::Arc { radius, half_angle } => {
                circle(z.at, z.dir, radius.0, Sector::new(z.dir, *half_angle))
            }
            ZoneShape::Line { length, width } => Line::new(z.dir, *length, *width).area(z.at),
        };
        swings.push(Swing {
            source: Source::Zone(i),
            owner: z.owner,
            slot: 0,
            friendly: z.friendly,
            area,
            damage: z.damage,
            damage_type: z.damage_type,
            rehit_ticks: None,
            breaks_charge: z.breaks_charge,
            smash: None,
            combo: false,
            poise_damage: None,
            stagger: None,
            rune: None,
            once: false,
        });
    }
}

fn circle(centre: Point, facing: Dir, radius: i32, arc: Option<Sector>) -> Area {
    Area {
        centre,
        facing,
        radius,
        arc,
        line: None,
    }
}

/// The hit box's centre: `forward` along `facing` and `right` a quarter turn clockwise from it.
pub(super) fn centre(at: Point, facing: Dir, forward: i32, right: i32) -> Point {
    let (fx, fy) = (i64::from(facing.x), i64::from(facing.y));
    let (fw, rt) = (i64::from(forward), i64::from(right));
    let scale = i64::from(trig::SCALE);
    let dx = (fx * fw - fy * rt) / scale;
    let dy = (fy * fw + fx * rt) / scale;
    let move_by = |v: Fx, d: i64| Fx(i32::try_from(i64::from(v.0) + d).unwrap_or(v.0));
    Point {
        x: move_by(at.x, dx),
        y: move_by(at.y, dy),
    }
}

/// Whether the swing's hit box overlaps the circle at `at` with `radius`.
fn covers(w: &Swing, at: Point, radius: i32) -> bool {
    w.area.covers(at, radius)
}

impl Area {
    pub(super) fn covers(&self, at: Point, radius: i32) -> bool {
        if !overlaps(self.centre, self.radius, at, radius) {
            return false;
        }
        if self.arc.is_none() && self.line.is_none() {
            return true;
        }
        let d = (
            i128::from(at.x.0) - i128::from(self.centre.x.0),
            i128::from(at.y.0) - i128::from(self.centre.y.0),
        );
        let Some(arc) = &self.arc else {
            return self
                .line
                .as_ref()
                .is_some_and(|line| line.touches(d, radius));
        };
        // A centre within the angle and within reach overlaps; one outside the angle overlaps only
        // if its circle crosses one of the two edges, the nearest part of the sector to it.
        within_angle(d, self.facing, arc.cos)
            || arc
                .edges
                .iter()
                .any(|&edge| touches_edge(d, edge, self.radius, radius))
    }
}

/// Whether `d` is at most the half angle whose cosine is `cos` from `facing`:
/// `d · f ≥ |d| |f| cos`, squared so no root is taken. At the centre itself, it is.
fn within_angle(d: (i128, i128), facing: Dir, cos: i32) -> bool {
    let (fx, fy) = (i128::from(facing.x), i128::from(facing.y));
    let dot = d.0 * fx + d.1 * fy;
    let d2 = d.0 * d.0 + d.1 * d.1;
    if d2 == 0 {
        return true;
    }
    let cos = i128::from(cos);
    let scale = i128::from(trig::SCALE);
    let lhs = dot * scale * (dot * scale);
    let rhs = cos * cos * d2 * (fx * fx + fy * fy);
    if cos >= 0 {
        dot >= 0 && lhs >= rhs
    } else {
        dot >= 0 || lhs <= rhs
    }
}

/// Whether the circle at `d` (from the arc's centre) with `radius` reaches the edge running
/// `length` along `edge`, a direction of length `trig::SCALE`.
fn touches_edge(d: (i128, i128), edge: Dir, length: i32, radius: i32) -> bool {
    let (ex, ey) = (i128::from(edge.x), i128::from(edge.y));
    let scale = i128::from(trig::SCALE);
    let r2 = i128::from(radius) * i128::from(radius);
    let along = d.0 * ex + d.1 * ey;
    if along <= 0 {
        return d.0 * d.0 + d.1 * d.1 <= r2;
    }
    let len = i128::from(length);
    if along >= len * scale {
        let (px, py) = (d.0 - len * ex / scale, d.1 - len * ey / scale);
        return px * px + py * py <= r2;
    }
    let across = d.0 * ey - d.1 * ex;
    across * across <= r2 * scale * scale
}

#[cfg(test)]
mod party_tests;
