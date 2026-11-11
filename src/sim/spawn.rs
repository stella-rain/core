//! Spawning of the live rules: waves, the view edge and the enemy's start.

use super::{Area, World, agents, count, scale_hp};
use crate::behaviour::BossCall;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::rng::SplitMix64;
use crate::sim::state::{AgentKey, AgentSource, State};
use crate::sim::state::{PlayerIdx, Players};
use crate::stage::{Side, ViewEdge};

pub(super) fn spawn_waves(stage: &World, s: &mut State) {
    if stage.director {
        return spawn_waves_directed(stage, s);
    }
    // The party's box, built on the first due wave of the tick: players do not move meanwhile.
    let mut area = None;
    for (i, wave) in stage.waves.iter().enumerate() {
        let spawned = s.spawned[i];
        if spawned >= wave.count {
            continue;
        }
        let due = u64::from(wave.at_tick) + u64::from(spawned) * u64::from(wave.every_ticks);
        if u64::from(s.tick) < due {
            continue;
        }
        spawn_wave(stage, s, i, &mut area);
    }
}

/// The spawn director (DIR1, DS103): the waves of a horde run, held at the crowd target.
///
/// A wave is due when its tick comes, as it is without the director, plus what it has been held
/// so far (`Horde::held`): a held spawn is not dropped and not queued, its schedule slides. The
/// room is the target less the enemies alive, taken once. If every due wave fits, all spawn in
/// index order, drawing in that order as they do without the director, so a run under the target
/// is unchanged. If not, the waves held longest spawn (ties to the lower index) in index order,
/// and every other due wave is held one more tick and counted in `Load::held_spawns`. A held
/// spawn draws no angle and does not count as spawned. The headroom between the target and the
/// cap stays for the specials, which the cap alone limits.
fn spawn_waves_directed(stage: &World, s: &mut State) {
    use crate::time_limit::MAX_WAVES;
    let mut due = [0usize; MAX_WAVES];
    let mut n = 0;
    if let Some(horde) = s.horde.as_ref() {
        for (i, wave) in stage.waves.iter().enumerate() {
            let spawned = s.spawned[i];
            if spawned >= wave.count || n == MAX_WAVES {
                continue;
            }
            let slid = u64::from(horde.held.get(i).copied().unwrap_or(0));
            let at = u64::from(wave.at_tick) + u64::from(spawned) * u64::from(wave.every_ticks);
            if u64::from(s.tick) >= at + slid {
                due[n] = i;
                n += 1;
            }
        }
    }
    let room = (stage.crowd_target as usize).saturating_sub(s.enemies.len());
    let mut spawn = [true; MAX_WAVES];
    if n > room {
        // The `room` waves held longest, the lower index first among equals.
        let held = |i: usize| s.horde.as_ref().map_or(0, |h| h.held[i]);
        let mut order = [0usize; MAX_WAVES];
        for (p, slot) in order.iter_mut().enumerate().take(n) {
            *slot = p;
        }
        order[..n].sort_unstable_by_key(|&p| (std::cmp::Reverse(held(due[p])), due[p]));
        for &p in &order[room..n] {
            spawn[p] = false;
        }
    }
    let mut area = None;
    for p in 0..n {
        let i = due[p];
        if !spawn[p] {
            if let Some(horde) = s.horde.as_mut() {
                horde.held[i] = horde.held[i].saturating_add(1);
            }
            s.scratch.held_spawns = s.scratch.held_spawns.saturating_add(1);
            continue;
        }
        spawn_wave(stage, s, i, &mut area);
    }
}

/// One spawn of wave `i`: counted as spawned, placed, and refused only by the caps.
fn spawn_wave(stage: &World, s: &mut State, i: usize, area: &mut Option<Area>) {
    let wave = &stage.waves[i];
    s.spawned[i] += 1;
    let key = AgentKey {
        source: AgentSource::Wave,
        index: u32::try_from(i).unwrap_or(u32::MAX),
        rule: 0,
    };
    let at = match (wave.edge, wave.spawn) {
        (Some(edge), _) => {
            let area = *area.get_or_insert_with(|| Area::of_view(&stage.map(), &s.players));
            edge_point(stage, area, &mut s.rng.spawn, edge)
        }
        (None, Some(p)) => p,
        // The validator refuses a wave with neither: the lowest up player's place.
        (None, None) => {
            s.players[Players::slots(s.players.targets())
                .next()
                .unwrap_or(PlayerIdx::SOLO)]
            .at
        }
    };
    spawn_enemy(stage, s, key, at);
}

/// One side of the spawn rectangle: `fixed` is its coordinate on the axis it stands on, and it
/// runs along the other axis from `lo` to `hi`. The sides come in the order top, right, bottom,
/// left; the opposite of side `i` is `(i + 2) % 4`.
#[derive(Clone, Copy)]
pub(super) struct RectSide {
    vertical: bool,
    fixed: i64,
    lo: i64,
    hi: i64,
}

impl RectSide {
    /// The part of the side on the map `w` by `h`, as `(lo, hi)`, if it has a length.
    pub(super) fn on_map(self, w: i64, h: i64) -> Option<(i64, i64)> {
        let (across, along) = if self.vertical { (w, h) } else { (h, w) };
        if !(0..=across).contains(&self.fixed) {
            return None;
        }
        let (lo, hi) = (self.lo.max(0), self.hi.min(along));
        (hi > lo).then_some((lo, hi))
    }

    /// The point `t` along the side from `lo`.
    fn at(self, lo: i64, t: i64) -> (i64, i64) {
        if self.vertical {
            (self.fixed, lo + t)
        } else {
            (lo + t, self.fixed)
        }
    }
}

/// The sides the `any` draw walks, in the order top, right, bottom, left, each with its part:
/// top and right run `[lo, hi)` and bottom and left `(lo, hi]`, so every perimeter point has one
/// draw value (the corners go top-left to top, top-right to right, bottom-right to bottom,
/// bottom-left to left). The part is the part on the map `w` by `h`; with none on the map, the whole sides (moved onto it by the
/// caller).
fn any_parts(
    px: i64,
    py: i64,
    ex: i64,
    ey: i64,
    w: i64,
    h: i64,
) -> [(RectSide, Option<(i64, i64)>); 4] {
    let mut sides = rect_sides(px, py, ex, ey);
    // Each corner belongs to one side: top owns top-left, right owns top-right, bottom owns
    // bottom-right and left owns bottom-left, so the four walk the perimeter once.
    for side in &mut sides[2..] {
        side.lo += 1;
        side.hi += 1;
    }
    let parts = sides.map(|side| side.on_map(w, h));
    let on_map = parts.iter().flatten().map(|(lo, hi)| hi - lo).sum::<i64>() > 0;
    let mut out = sides.map(|side| (side, None));
    for (i, side) in sides.iter().enumerate() {
        out[i].1 = if on_map {
            parts[i]
        } else {
            Some((side.lo, side.hi))
        };
    }
    out
}

/// How many values the `any` draw has: the length of the on-map parts of all four sides.
pub(super) fn any_total(px: i64, py: i64, ex: i64, ey: i64, w: i64, h: i64) -> i64 {
    any_parts(px, py, ex, ey, w, h)
        .iter()
        .filter_map(|(_, part)| *part)
        .map(|(lo, hi)| hi - lo)
        .sum()
}

/// The point of the `any` draw's value `t`: the walk along the parts in side order.
pub(super) fn any_point(
    px: i64,
    py: i64,
    ex: i64,
    ey: i64,
    w: i64,
    h: i64,
    mut t: i64,
) -> (i64, i64) {
    for (side, part) in any_parts(px, py, ex, ey, w, h) {
        let Some((lo, hi)) = part else { continue };
        if t < hi - lo {
            return side.at(lo, t);
        }
        t -= hi - lo;
    }
    (px, py)
}

/// The four sides of the spawn rectangle around (`px`, `py`).
pub(super) fn rect_sides(px: i64, py: i64, ex: i64, ey: i64) -> [RectSide; 4] {
    [
        RectSide {
            vertical: false,
            fixed: py - ey,
            lo: px - ex,
            hi: px + ex,
        },
        RectSide {
            vertical: true,
            fixed: px + ex,
            lo: py - ey,
            hi: py + ey,
        },
        RectSide {
            vertical: false,
            fixed: py + ey,
            lo: px - ex,
            hi: px + ex,
        },
        RectSide {
            vertical: true,
            fixed: px - ex,
            lo: py - ey,
            hi: py + ey,
        },
    ]
}

/// Which side to use when side `i` (0 top, 1 right, 2 bottom, 3 left) of a rectangle around
/// the view centre is wanted: `i` if on the map, otherwise the opposite side `(i + 2) % 4` if
/// on the map; if neither fits, `i` is kept (and clamped to the map border).
pub(super) fn usable_side(parts: &[Option<(i64, i64)>; 4], i: usize) -> usize {
    if parts[i].is_some() || parts[(i + 2) % 4].is_none() {
        i
    } else {
        (i + 2) % 4
    }
}

/// A point on the spawn rectangle around the view's centre for a main character at `from` (VS1,
/// `view_centre`): ±the spawn edge (the view and `spawn_margin`), drawn
/// from `rng` (the wave's stream for a wave, the horde's for an elite) on the part of `edge`'s
/// side that is on the map. A side wholly off the map gives way to the opposite one; with
/// neither on the map the side itself is used and the point moved onto the map. `any` is one draw
/// over the on-map parts of all four sides, each corner on one side only (`any_parts`); a side with no
/// `along` is one draw along its part; with `along` there is no draw.
pub(super) fn edge_point(stage: &World, area: Area, rng: &mut SplitMix64, edge: ViewEdge) -> Point {
    let map = stage.map();
    let (w, h) = (i64::from(map.width.0), i64::from(map.height.0));
    let centre = area.centre;
    let (px, py) = (i64::from(centre.x.0), i64::from(centre.y.0));
    let (ex, ey) = (
        i64::from(stage.tuning.spawn_edge_x.0) + area.half_x,
        i64::from(stage.tuning.spawn_edge_y.0) + area.half_y,
    );
    let sides = rect_sides(px, py, ex, ey);
    let parts = sides.map(|side| side.on_map(w, h));
    let draw =
        |rng: &mut SplitMix64, n: i64| i64::from(rng.below(u32::try_from(n).unwrap_or(u32::MAX)));
    let (x, y) = match edge.side {
        Side::Any => {
            let t = draw(rng, any_total(px, py, ex, ey, w, h));
            any_point(px, py, ex, ey, w, h, t)
        }
        named => {
            let i = match named {
                Side::Top => 0,
                Side::Right => 1,
                Side::Bottom => 2,
                // `Any` is the arm above.
                Side::Left | Side::Any => 3,
            };
            let j = usable_side(&parts, i);
            let side = sides[j];
            let (lo, hi) = parts[j].unwrap_or((side.lo, side.hi));
            match edge.along {
                Some(along) => {
                    let (half, middle) = if side.vertical { (ey, py) } else { (ex, px) };
                    let t = middle + i64::from(along) * half / 1000;
                    if side.vertical {
                        (side.fixed, t)
                    } else {
                        (t, side.fixed)
                    }
                }
                None => side.at(lo, draw(rng, hi - lo)),
            }
        }
    };
    let fx = |v: i64| Fx(i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX }));
    map.clamp(Point { x: fx(x), y: fx(y) })
}

/// Spawns an enemy of `def` at `at`, unless the cap on enemies alive is reached, or for an
/// elite the stage's cap on elites; an elite that comes after the boss's call is called at
/// once. Returns whether it spawned.
pub(super) fn spawn_enemy(stage: &World, s: &mut State, key: AgentKey, at: Point) -> bool {
    let index = match key.source {
        AgentSource::BossRule => {
            let boss = s.boss.as_ref().expect("a boss summons its own agents");
            stage.boss_summon(boss.boss, key.index, key.rule)
        }
        _ => stage.defs.of_key(key),
    };
    let d = stage.defs.get(index);
    let def = &d.agent;
    // Every number below `MAX_ENTITY_NUMBER` is an enemy's or a companion's; from there on a
    // number reads as a rejoined player's, so the spawns stop (a run spawns far fewer).
    let room = count(&s.enemies) < stage.enemy_cap
        && s.next_id < crate::id::MAX_ENTITY_NUMBER
        && (def.elite.is_none()
            || s.enemies.iter().filter(|e| e.elite.is_some()).count()
                < usize::from(stage.elite_cap()));
    if !room {
        s.scratch.refused_spawns = s.scratch.refused_spawns.saturating_add(1);
        return false;
    }
    let id = EntityId(s.next_id);
    s.next_id += 1;
    let mut enemy = agents::new_agent(id, index, d, key, at, 0);
    enemy.hp = scale_hp(enemy.hp, s.hp_pct);
    enemy.max_hp = enemy.hp;
    if let Some(el) = enemy.elite.as_mut() {
        let called = s.boss.as_ref().and_then(|b| b.elites_called) == Some(true);
        match (el.boss_call, enemy.leave.as_mut()) {
            (BossCall::Leave, Some(l)) => l.go = called,
            // A shrine, or any elite that stays, is never called.
            (BossCall::Stay, _) => {}
            // A mimic is called when it reveals, not before.
            _ if el.disguised => {}
            _ => el.called = called,
        }
    }
    s.enemies.push(enemy);
    // A pack leader's escort comes with it, on a ring around it. Only a wave's enemy
    // has one: that is the definition the escort is found by.
    if key.source == AgentSource::Wave {
        if let Some(escort) = def.elite.as_ref().and_then(|e| e.escort.as_ref()) {
            let key = AgentKey {
                source: AgentSource::Escort,
                index: key.index,
                rule: 0,
            };
            agents::ring_up(stage, s, key, escort, at);
        }
    }
    true
}
