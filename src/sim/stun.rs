//! Stun: what a smash's stun does to the enemy it hits. The status is `add_stun`'s
//! (`entity.rs`). Only an enemy takes a stun: a boss and its parts resist it, and the main
//! character's and the companions' hits never stun anything. An elite takes its share of the
//! ticks, and a stun of no ticks does nothing.
//!
//! Every stagger fill goes through `fill`: the counter bonus's factor and then the breaker's
//! scale the value, the refill percent of the bars already broken applies, and a full gauge
//! staggers the target. The breaker's burst (`player::stagger_burst`) goes off right after it,
//! with the damage of the hit that filled, so one stagger bursts once.

use super::World;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::limits::MAX_STAGGER_BREAKS;
use crate::sim::PlayerIdx;
use crate::sim::state::{AgentState, Stagger, State, StatusKind, StatusState};
use crate::sim::world::{AttackIndex, Tuning};
use crate::validate::limits::MAX_STUN_TICKS;

use super::attacks;
use super::entity::Who;

/// Whether `statuses` holds a stun.
pub(super) fn stunned(statuses: &[StatusState]) -> bool {
    statuses.iter().any(|st| st.kind == StatusKind::Stun)
}

/// What the n-th hard control on one enemy does (C42), when the rules name a list: the ticks it
/// keeps, or `None` when it is refused. Past the list the enemy turns immune for the rules'
/// `control_immune_ticks` and its count starts over; a control scaled to 0 ticks does not land
/// and does not count. A landing one counts and restarts the reset. Knockdown reuses it.
pub(super) fn land_hard_control(e: &mut AgentState, tuning: &Tuning, ticks: u32) -> Option<u32> {
    let n = usize::from(e.control.hard_controls);
    if n >= usize::from(tuning.control_repeat_len) {
        e.stun_immune = tuning.control_immune_ticks;
        e.control.hard_controls = 0;
        e.control.hard_control_reset = 0;
        return None;
    }
    let ticks = ticks * u32::from(tuning.control_repeat_pcts[n]) / 100;
    if ticks == 0 {
        return None;
    }
    e.control.hard_controls += 1;
    e.control.hard_control_reset = tuning.control_reset_ticks;
    Some(ticks)
}

/// Stuns `target` for a smash's `ticks`: all of them on a regular enemy, the elite's share on an
/// elite, nothing on any other target. With a hard-control list in the rules each further stun
/// on one enemy is shorter and then refused (`land_hard_control`); a stun that lands ends a
/// flinch and, when the rules say so, puts `vulnerable` on its target for its ticks.
pub(super) fn apply(
    stage: &World,
    s: &mut State,
    target: Who,
    source: EntityId,
    ticks: u32,
    events: &mut Vec<DomainEvent>,
) {
    apply_hit(stage, s, target, source, (ticks, 0), events);
}

/// `apply` for a stun that comes from a hit of `hit` damage (`(ticks, hit)`): a fill the resisted
/// share credits can stagger the target, and the stagger burst takes a percent of `hit`.
fn apply_hit(
    stage: &World,
    s: &mut State,
    target: Who,
    source: EntityId,
    (ticks, hit): (u32, u32),
    events: &mut Vec<DomainEvent>,
) {
    // C04: a stun an elite or the boss resists fills its gauge, before the immunity test (an
    // immune elite resisted it too). The boss and a part resist it whole, an elite the share
    // its stun does not land.
    if stage.has_stagger {
        if let Some(me) = s.players.of_id(source) {
            let value = stage.tuning.stagger_values[0];
            let resisted = match target {
                Who::Boss | Who::BossPart(_) => 100,
                Who::Enemy(i) if s.enemies[i].elite.is_some() => {
                    100u32.saturating_sub(s.stun.elite_pct.min(100))
                }
                _ => 0,
            };
            fill(
                stage,
                s,
                target,
                me,
                value.saturating_mul(resisted) / 100,
                hit,
                events,
            );
        }
    }
    let Who::Enemy(i) = target else {
        return;
    };
    if s.enemies[i].stun_immune > 0 {
        return;
    }
    let ticks = ticks.min(MAX_STUN_TICKS);
    let elite = stage.defs.agent(s.enemies[i].def).elite.is_some();
    let ticks = if elite {
        ticks.saturating_mul(s.stun.elite_pct) / 100
    } else {
        ticks
    };
    let ticks = if ticks > 0 && stage.tuning.control_repeat_len > 0 {
        land_hard_control(&mut s.enemies[i], &stage.tuning, ticks).unwrap_or(0)
    } else {
        ticks
    };
    if ticks > 0 {
        // The stun cancels what the enemy was doing: its running attack, in any phase. Orbs it
        // has spawned fly on.
        let id = s.id_of(target);
        s.add_stun(target, source, ticks, events);
        attacks::cancel_runs_of(stage, s, id);
        end_flinch(stage, s, i, events);
        let pct = stage.tuning.vulnerable_stun_pct;
        if pct > 0 {
            s.add_status(target, StatusKind::Vulnerable, source, pct, ticks, events);
        }
    }
}

/// The counter (C05): a smash step of `me`'s hit `target`, an enemy, an elite, the boss or a part,
/// during a `counterable` wind-up of `attack` (found before the hit by `attacks::counterable_run`).
/// The attack is cancelled (the one cancel every control uses), `Countered` is reported, a regular
/// enemy or an elite is stunned for the rules' `counter_stun_ticks` through `apply` (the elite's
/// share, the control list and `vulnerable` all apply), and an elite or the boss gains
/// `stagger_values.counter`. A target the hit killed is not countered.
#[inline(never)]
pub(super) fn counter(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    target: Who,
    attack: AttackIndex,
    (stun_ticks, hit): (u32, u32),
    events: &mut Vec<DomainEvent>,
) {
    let target = match target {
        Who::BossPart(_) => Who::Boss,
        other => other,
    };
    if !s.is_alive(target) {
        return;
    }
    let owner = s.id_of(target);
    let source = s.players.id(me);
    attacks::cancel_runs_of(stage, s, owner);
    events.push(DomainEvent::Countered {
        attacker: owner,
        player: me,
        attack: stage.attacks[usize::from(attack.0)].preset_id.clone(),
    });
    let t = &stage.tuning;
    if matches!(target, Who::Enemy(_)) && stun_ticks > 0 {
        apply_hit(stage, s, target, source, (stun_ticks, hit), events);
    }
    if t.stagger_counter > 0 {
        fill(stage, s, target, me, t.stagger_counter, hit, events);
    }
}

/// What a combo hit of `me` on `target` does after the damage: the counter of a counterable
/// wind-up (`countered`) and the stun of a taken counter bonus. A hit that does both stuns once,
/// for the longer of the two, so the control list counts one stun for the hit. The boss and a
/// part take only the resisted share fill, as for any stun. A target the hit killed is not
/// stunned. Only a combo hit carries the bonus.
#[inline(never)]
pub(super) fn counter_bonus_stun(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    (target, combo): (Who, bool),
    countered: Option<AttackIndex>,
    hit: u32,
    events: &mut Vec<DomainEvent>,
) {
    let own = stage.tuning.counter_stun_ticks;
    let bonus = if combo {
        super::player::counter_taken(stage, s, me).map_or(0, |g| g.stun_ticks)
    } else {
        0
    };
    if let Some(a) = countered {
        counter(stage, s, me, target, a, (own.max(bonus), hit), events);
    }
    let target = match target {
        Who::BossPart(_) => Who::Boss,
        other => other,
    };
    // A counter has applied the stun to an enemy already; the boss never takes one but its fill.
    let applied = countered.is_some() && matches!(target, Who::Enemy(_));
    if bonus > 0 && !applied && s.is_alive(target) {
        let source = s.players.id(me);
        apply_hit(stage, s, target, source, (bonus, hit), events);
    }
}

/// Ends enemy `i`'s flinch, if it has one: its immunity starts, as at every end.
fn end_flinch(stage: &World, s: &mut State, i: usize, events: &mut Vec<DomainEvent>) {
    let e = &mut s.enemies[i];
    if e.control.flinch > 0 {
        e.control.flinch = 0;
        e.control.flinch_immune = stage.tuning.flinch_immune_ticks;
        events.push(DomainEvent::FlinchEnded { target: e.id });
    }
}

/// Makes enemy `i` flinch when a hit of `poise_damage` reaches its poise (C02): it must name
/// one, the rules a `flinch_ticks`, and the enemy must be alive, in plain sight, not stunned,
/// not on the ground, not flinching and not flinch-immune. The flinch cancels its running
/// attack and is neither a hard control nor `vulnerable`.
pub(super) fn flinch(
    stage: &World,
    s: &mut State,
    i: usize,
    poise_damage: u32,
    source: EntityId,
    events: &mut Vec<DomainEvent>,
) {
    let ticks = stage.tuning.flinch_ticks;
    let e = &s.enemies[i];
    if ticks == 0
        || e.hp == 0
        || e.shrine.is_some()
        || e.hidden()
        || e.control.flinch > 0
        || e.control.flinch_immune > 0
        || e.control.knocked_down > 0
        || stunned(&e.statuses)
        || (stage.has_stagger && e.elite.as_ref().is_some_and(|el| el.stagger.staggered > 0))
    {
        return;
    }
    match stage.defs.agent(e.def).poise {
        Some(poise) if poise <= poise_damage => {}
        _ => return,
    }
    let id = e.id;
    s.enemies[i].control.flinch = ticks;
    attacks::cancel_runs_of(stage, s, id);
    events.push(DomainEvent::StatusApplied {
        target: id,
        status: super::snapshot::FLINCH_ID.clone(),
        stacks: 1,
        duration_ticks: u32::from(ticks),
        source: s.players.of_id(source).map(|p| s.players.id(p)),
    });
}

/// The gauge numbers of `target`: its `stagger_max` and the ticks of its stagger (its own, else
/// the rules'). A part has none of its own: the boss's. `None` for a target with no gauge.
pub(super) fn stagger_of(stage: &World, s: &State, target: Who) -> Option<(u32, u16)> {
    let (max, own) = match target {
        Who::Boss | Who::BossPart(_) => {
            let def = &stage.boss(s.boss.as_ref()?.boss).boss;
            (def.stagger_max?, def.staggered_ticks)
        }
        Who::Enemy(i) => {
            let e = &s.enemies[i];
            e.elite.as_ref()?;
            let def = stage.defs.agent(e.def);
            (def.stagger_max?, def.staggered_ticks)
        }
        _ => return None,
    };
    Some((max, own.unwrap_or(stage.tuning.staggered_ticks)))
}

/// The gauge of `target`, which must have one.
fn gauge_of(s: &mut State, target: Who) -> Option<&mut Stagger> {
    match target {
        Who::Boss | Who::BossPart(_) => s.boss.as_mut().map(|b| &mut b.stagger),
        Who::Enemy(i) => s.enemies[i].elite.as_mut().map(|e| &mut e.stagger),
        _ => None,
    }
}

/// Whether `target` is staggered now: the boss, a part (the boss's) or an elite.
pub(super) fn is_staggered(s: &State, target: Who) -> bool {
    match target {
        Who::Boss | Who::BossPart(_) => s.boss.as_ref().is_some_and(|b| b.stagger.staggered > 0),
        Who::Enemy(i) => s.enemies[i]
            .elite
            .as_ref()
            .is_some_and(|e| e.stagger.staggered > 0),
        _ => false,
    }
}

/// The percent `vulnerable` and the staggered target's extra damage make together, in place of
/// `vulnerable` alone: kinds multiply, `(100 + v) * (100 + d) / 100 - 100` (C03).
pub(super) fn raised(stage: &World, s: &State, target: Who, vulnerable: u32) -> u32 {
    let pct = stage.tuning.stagger_damage_pct;
    if pct == 0 || !is_staggered(s, target) {
        return vulnerable;
    }
    let both = (100 + u64::from(vulnerable)) * (100 + u64::from(pct)) / 100 - 100;
    u32::try_from(both).unwrap_or(u32::MAX)
}

/// The percent `vulnerable` and the down bonus make together, in place of `vulnerable` alone, for
/// a smash step's hit on a downed enemy: kinds multiply, `(100 + v) * (100 + d) / 100 - 100` (C09).
/// Any other target, or an enemy standing, keeps `vulnerable`.
pub(super) fn down_raised(stage: &World, s: &State, target: Who, vulnerable: u32) -> u32 {
    let pct = stage.tuning.down_bonus_pct;
    let Who::Enemy(i) = target else {
        return vulnerable;
    };
    if pct == 0 || s.enemies[i].control.knocked_down == 0 {
        return vulnerable;
    }
    let both = (100 + u64::from(vulnerable)) * (100 + u64::from(pct)) / 100 - 100;
    u32::try_from(both).unwrap_or(u32::MAX)
}

/// Adds `value` (scaled by the refill list for the bars already broken) to the gauge of `target`,
/// and staggers it when the gauge is full. It does nothing without a gauge, for a dead, staggered
/// or invulnerable (a boss in transition) target, for a value that scales to nothing, or for a
/// player who is not up. A part's hit fills the boss's gauge (C12). `hit` is the damage of the hit
/// being resolved: the breaker's burst, when this fill staggers the target, takes a percent of it.
pub(super) fn fill(
    stage: &World,
    s: &mut State,
    target: Who,
    me: PlayerIdx,
    value: u32,
    hit: u32,
    events: &mut Vec<DomainEvent>,
) {
    if !stage.has_stagger || value == 0 || !s.players[me].is_up() {
        return;
    }
    let target = if let Who::BossPart(_) = target {
        Who::Boss
    } else {
        target
    };
    let Some((max, ticks)) = stagger_of(stage, s, target) else {
        return;
    };
    let alive = match target {
        Who::Boss => s
            .boss
            .as_ref()
            .is_some_and(|b| b.hp > 0 && b.invulnerable == 0),
        Who::Enemy(i) => s.enemies[i].hp > 0,
        _ => false,
    };
    if !alive {
        return;
    }
    // A step that took the counter bonus scales every fill credited to its player.
    let value = match super::player::counter_taken(stage, s, me) {
        Some(fx) if fx.stagger_pct > 100 => value.saturating_mul(fx.stagger_pct) / 100,
        _ => value,
    };
    // The breaker's factor composes with it, the counter's first, each truncated.
    let value = super::player::breaker_fill(stage, s, me, value);
    let t = &stage.tuning;
    let Some(g) = gauge_of(s, target) else {
        return;
    };
    if g.staggered > 0 {
        return;
    }
    let pct = if t.stagger_refill_len == 0 {
        100
    } else {
        let last = usize::from(t.stagger_refill_len) - 1;
        u32::from(t.stagger_refill_pcts[usize::from(g.breaks).min(last)])
    };
    let add = value.saturating_mul(pct) / 100;
    if add == 0 {
        return;
    }
    g.gauge = g.gauge.saturating_add(add).min(max);
    g.drain = t.stagger_drain_delay;
    if g.gauge >= max {
        stagger(stage, s, target, s.players.id(me), ticks, events);
        // The breaker's burst goes off in the call that staggers the target, once.
        if let Some(burst) = stage.breaker_of(s, me).and_then(|g| g.burst) {
            super::player::stagger_burst(stage, s, me, (target, hit), burst, events);
        }
    }
}

/// Staggers `target` (the boss or an elite) for `ticks`: its gauge empties, its next bar fills
/// slower, its attacks end, and it reports the status and, when the rules say so, takes
/// `vulnerable` for the stagger's ticks. It is no hard control (C03).
fn stagger(
    stage: &World,
    s: &mut State,
    target: Who,
    source: EntityId,
    ticks: u16,
    events: &mut Vec<DomainEvent>,
) {
    if let Some(g) = gauge_of(s, target) {
        g.staggered = ticks;
        g.gauge = 0;
        g.drain = 0;
        g.breaks = g.breaks.saturating_add(1).min(MAX_STAGGER_BREAKS);
    }
    let id = s.id_of(target);
    attacks::cancel_runs_of(stage, s, id);
    events.push(DomainEvent::StatusApplied {
        target: id,
        status: super::snapshot::STAGGERED_ID.clone(),
        stacks: 1,
        duration_ticks: u32::from(ticks),
        source: Some(source),
    });
    let pct = stage.tuning.vulnerable_stagger_pct;
    if pct > 0 {
        s.add_status(
            target,
            StatusKind::Vulnerable,
            source,
            pct,
            u32::from(ticks),
            events,
        );
    }
}

/// One tick of every gauge: a stagger counts down and reports its end; an idle gauge waits out its
/// drain delay and then loses the rules' amount a tick (the tick after the delay is the first to
/// drain). A gauge that holds nothing costs one compare.
pub(super) fn stagger_tick(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let per_tick = stage.tuning.stagger_drain_per_tick;
    fn tick(g: &mut Stagger, id: EntityId, per_tick: u32, events: &mut Vec<DomainEvent>) {
        if g.staggered > 0 {
            g.staggered -= 1;
            if g.staggered == 0 {
                events.push(DomainEvent::StaggerEnded { target: id });
            }
        } else if g.drain > 0 {
            g.drain -= 1;
        } else if g.gauge > 0 && per_tick > 0 {
            g.gauge = g.gauge.saturating_sub(per_tick);
        }
    }
    if let Some(b) = s.boss.as_mut() {
        tick(&mut b.stagger, b.id, per_tick, events);
    }
    for e in &mut s.enemies {
        let id = e.id;
        if let Some(el) = e.elite.as_mut() {
            tick(&mut el.stagger, id, per_tick, events);
        }
    }
}

/// Knocks enemy `i` down when the knockback of `me`'s smash lands on it (C09): it must be a living
/// regular enemy (no elite) and `me` must be up. A fresh knockdown keeps the rules' `down_ticks`
/// (through the hard-control list when one is named, as a stun does, and is refused past it or
/// while the enemy is stun-immune), cancels its running attack, ends a flinch and reports the
/// status, with `vulnerable` when the rules say so. It is a hard control and no stun. A knockdown
/// on a downed enemy counts nothing and only lengthens it, never past `down_ticks`.
pub(super) fn knock_down(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    i: usize,
    events: &mut Vec<DomainEvent>,
) {
    if !s.players[me].is_up() {
        return;
    }
    let t = &stage.tuning;
    let source = s.players.id(me);
    let e = &mut s.enemies[i];
    if e.hp == 0 || e.elite.is_some() {
        return;
    }
    let id = e.id;
    let fresh = e.control.knocked_down == 0;
    let ticks = if fresh {
        if e.stun_immune > 0 {
            return;
        }
        let ticks = u32::from(t.down_ticks);
        if t.control_repeat_len > 0 {
            let Some(ticks) = land_hard_control(e, t, ticks) else {
                return;
            };
            ticks
        } else {
            ticks
        }
    } else {
        // Counts nothing: the entry of the count the first knockdown made.
        let pct = if t.control_repeat_len > 0 {
            let last = usize::from(t.control_repeat_len) - 1;
            let n = usize::from(e.control.hard_controls.saturating_sub(1)).min(last);
            u32::from(t.control_repeat_pcts[n])
        } else {
            100
        };
        let ticks = u32::from(t.down_ticks) * pct / 100;
        if ticks <= u32::from(e.control.knocked_down) {
            return;
        }
        ticks
    };
    e.control.knocked_down = u16::try_from(ticks)
        .unwrap_or(t.down_ticks)
        .min(t.down_ticks);
    if fresh {
        end_flinch(stage, s, i, events);
        attacks::cancel_runs_of(stage, s, id);
    }
    events.push(DomainEvent::StatusApplied {
        target: id,
        status: super::snapshot::KNOCKED_DOWN_ID.clone(),
        stacks: 1,
        duration_ticks: ticks,
        source: Some(source),
    });
    let pct = t.vulnerable_down_pct;
    if pct == 0 {
        return;
    }
    // A lengthened knockdown lengthens the `vulnerable` it put and does not stack it.
    let held = s.enemies[i]
        .statuses
        .iter_mut()
        .find(|st| st.kind == StatusKind::Vulnerable);
    match held {
        Some(st) if !fresh => {
            if ticks > st.remaining {
                st.total = ticks;
            }
            st.remaining = st.remaining.max(ticks);
        }
        _ => s.add_status(
            Who::Enemy(i),
            StatusKind::Vulnerable,
            source,
            pct,
            ticks,
            events,
        ),
    }
}

/// One tick of every knockdown: it counts down and, the tick it reaches 0, reports its end. With
/// no hard-control list named the run stats' stun immunity starts then (with a list its own
/// governs). A separate pass, so a run whose rules name no knockdown never pays per enemy.
pub(super) fn down_tick(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let immune = (stage.tuning.control_repeat_len == 0).then_some(s.stun.immune_ticks);
    for e in &mut s.enemies {
        if e.control.knocked_down > 0 {
            e.control.knocked_down -= 1;
            if e.control.knocked_down == 0 {
                if let Some(ticks) = immune {
                    e.stun_immune = ticks;
                }
                events.push(DomainEvent::KnockdownEnded { target: e.id });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The ranged token of a run a stun cancels (a zone-leaving run frees its token at once; a
    //! channelled one cools it), on a state built as `damage_tests` builds one.

    use super::*;
    use crate::attack::AttackRef;
    use crate::bench_fixtures::worst_case;
    use crate::engine::{Engine, Run};
    use crate::sim::PlayerIdx;
    use crate::sim::world::RangedPool;
    use serde_json::json;

    fn bare() -> (World, State) {
        let e: Engine = worst_case::warmed_up().into_party(1);
        let (w, s) = match e.run() {
            Run::V2 { world, state, .. } => (world, state),
            Run::V1 { .. } => panic!("the live rules"),
        };
        let mut s = s.clone();
        s.runs.clear();
        s.orbs.clear();
        s.zones.clear();
        s.echoes.clear();
        s.ranged_cooling.clear();
        (World::clone(w), s)
    }

    /// Starts `phases` as a ranged attack of enemy 0 and returns the world and state.
    fn ranged_run(phases: serde_json::Value) -> (World, State, RangedPool) {
        let (mut w, mut s) = bare();
        let attack: AttackRef = serde_json::from_value(json!({ "inline": phases })).unwrap();
        w.compile_attack(&attack, None).expect("compiles");
        let mut events = Vec::new();
        let target = Who::Player(PlayerIdx::new(0).unwrap());
        assert!(attacks::start(
            &w,
            &mut s,
            Who::Enemy(0),
            0,
            target,
            &attack,
            100,
            &mut events
        ));
        attacks::mark_ranged(&mut s);
        let pool = w.attacks[usize::from(s.runs[0].attack.0)].pool;
        (w, s, pool)
    }

    fn zone_leaving() -> serde_json::Value {
        json!([
            { "kind": "wind_up", "ticks": 5, "spawn": [
                { "type": "zone", "shape": { "type": "circle", "radius": 4096 },
                  "anchor": "target", "wind_up_ticks": 30, "active_ticks": 30 }] },
            { "kind": "active", "ticks": 3 },
            { "kind": "recovery", "ticks": 2 }
        ])
    }

    fn channelled() -> serde_json::Value {
        json!([
            { "kind": "wind_up", "ticks": 5 },
            { "kind": "active", "ticks": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1024 }, "radius": 4096 } },
            { "kind": "recovery", "ticks": 2 }
        ])
    }

    fn stun_owner(w: &World, s: &mut State) {
        let source = s.players.id(PlayerIdx::new(0).unwrap());
        s.enemies[0].stun_immune = 0;
        apply(w, s, Who::Enemy(0), source, 30, &mut Vec::new());
    }

    #[test]
    fn a_stun_frees_a_zone_leaving_runs_token_at_once_with_no_cooling() {
        let (w, mut s, _) = ranged_run(zone_leaving());
        assert!(!w.attacks[usize::from(s.runs[0].attack.0)].channelled);
        stun_owner(&w, &mut s);
        assert!(s.runs.is_empty(), "the run is gone");
        assert!(
            s.ranged_cooling.is_empty(),
            "no cooling: {:?}",
            s.ranged_cooling
        );
    }

    #[test]
    fn a_stun_cools_a_channelled_runs_token_once() {
        let (w, mut s, pool) = ranged_run(channelled());
        assert!(w.attacks[usize::from(s.runs[0].attack.0)].channelled);
        let until = s.tick + w.tuning.ranged_token_cooldown_ticks;
        stun_owner(&w, &mut s);
        assert!(s.runs.is_empty(), "the run is gone");
        assert_eq!(s.ranged_cooling.as_slice(), [(until, pool)]);
    }

    #[test]
    fn two_cancels_of_one_run_in_one_hit_pass_cool_its_token_once() {
        let (w, mut s, pool) = ranged_run(channelled());
        let until = s.tick + w.tuning.ranged_token_cooldown_ticks;
        let owner = s.runs[0].owner;
        s.scratch.ends_pinned = Some(s.runs.len());
        // The two asks, as a stun and a second one in the same pass make them.
        attacks::cancel_runs_of(&w, &mut s, owner);
        attacks::cancel_runs_of(&w, &mut s, owner);
        super::super::hit_boxes::apply_ends(&w, &mut s);
        assert!(s.runs.is_empty());
        assert_eq!(s.ranged_cooling.as_slice(), [(until, pool)]);
    }

    #[test]
    fn a_run_in_its_recovery_holds_no_token_and_cools_nothing() {
        let (w, mut s, _) = ranged_run(channelled());
        s.runs[0].phase = 2;
        stun_owner(&w, &mut s);
        assert!(s.runs.is_empty());
        assert!(s.ranged_cooling.is_empty(), "{:?}", s.ranged_cooling);
    }

    /// `phases` with its wind-up flagged counterable, run as a ranged attack of enemy 0.
    fn counterable_run(phases: serde_json::Value) -> (World, State, RangedPool) {
        let mut phases = phases;
        phases[0]["counterable"] = json!(true);
        let (mut w, s, pool) = ranged_run(phases);
        w.tuning.counter_stun_ticks = 40;
        (w, s, pool)
    }

    /// The counter of enemy 0 by player 0, as the hit pass makes it: deferred, then applied.
    fn counter_owner(w: &World, s: &mut State) {
        let me = PlayerIdx::new(0).unwrap();
        let attack = attacks::counterable_run(w, s, s.enemies[0].id).expect("a counterable run");
        s.enemies[0].stun_immune = 0;
        s.scratch.ends_pinned = Some(s.runs.len());
        let own = w.tuning.counter_stun_ticks;
        counter(w, s, me, Who::Enemy(0), attack, (own, 0), &mut Vec::new());
        super::super::hit_boxes::apply_ends(w, s);
        s.scratch.ends_pinned = None;
    }

    #[test]
    fn a_counter_frees_a_zone_leaving_runs_token_at_once_with_no_cooling() {
        let (w, mut s, _) = counterable_run(zone_leaving());
        counter_owner(&w, &mut s);
        assert!(s.runs.is_empty(), "the run is gone");
        assert!(
            s.ranged_cooling.is_empty(),
            "no cooling: {:?}",
            s.ranged_cooling
        );
        assert!(stunned(&s.enemies[0].statuses), "and it is stunned");
    }

    #[test]
    fn a_counter_cools_a_channelled_runs_token_once() {
        let (w, mut s, pool) = counterable_run(channelled());
        let until = s.tick + w.tuning.ranged_token_cooldown_ticks;
        counter_owner(&w, &mut s);
        assert!(s.runs.is_empty(), "the run is gone");
        assert_eq!(s.ranged_cooling.as_slice(), [(until, pool)]);
    }

    #[test]
    fn a_run_a_cancel_already_ended_in_the_pass_is_not_countered_again() {
        let (w, mut s, _) = counterable_run(channelled());
        let owner = s.enemies[0].id;
        assert!(attacks::counterable_run(&w, &s, owner).is_some());
        s.scratch.ends_pinned = Some(s.runs.len());
        attacks::cancel_runs_of(&w, &mut s, owner);
        assert!(attacks::counterable_run(&w, &s, owner).is_none());
        // A run that is not in a counterable wind-up is not countered either.
        s.scratch.ends_pending.clear();
        s.runs[0].phase = 1;
        assert!(attacks::counterable_run(&w, &s, owner).is_none());
    }

    #[test]
    fn any_player_s_smash_counters_and_two_on_one_tick_counter_once() {
        let e: Engine = worst_case::warmed_up().into_party(2);
        let (w, st) = match e.run() {
            Run::V2 { world, state, .. } => (world, state),
            Run::V1 { .. } => panic!("the live rules"),
        };
        let mut w = World::clone(w);
        let mut s = st.clone();
        s.runs.clear();
        s.ranged_cooling.clear();
        let mut phases = channelled();
        phases[0]["counterable"] = json!(true);
        let attack: AttackRef = serde_json::from_value(json!({ "inline": phases })).unwrap();
        w.compile_attack(&attack, None).expect("compiles");
        w.tuning.counter_stun_ticks = 40;
        let mut events = Vec::new();
        let target = Who::Player(PlayerIdx::new(0).unwrap());
        assert!(attacks::start(
            &w,
            &mut s,
            Who::Enemy(0),
            0,
            target,
            &attack,
            100,
            &mut events
        ));
        let owner = s.enemies[0].id;
        s.enemies[0].stun_immune = 0;
        s.scratch.ends_pinned = Some(s.runs.len());
        // Player 1's smash counters it, credited to player 1; player 0's, later in the same pass,
        // finds the cancel pending and counters nothing.
        let second = PlayerIdx::new(1).unwrap();
        let attack = attacks::counterable_run(&w, &s, owner).expect("counterable");
        events.clear();
        let own = w.tuning.counter_stun_ticks;
        counter(
            &w,
            &mut s,
            second,
            Who::Enemy(0),
            attack,
            (own, 0),
            &mut events,
        );
        assert!(attacks::counterable_run(&w, &s, owner).is_none());
        let countered: Vec<_> = events
            .iter()
            .filter_map(|ev| match ev {
                DomainEvent::Countered { player, .. } => Some(*player),
                _ => None,
            })
            .collect();
        assert_eq!(countered, [second]);
        super::super::hit_boxes::apply_ends(&w, &mut s);
        assert!(s.runs.is_empty());
    }

    #[test]
    fn the_shipped_zone_attacks_are_not_channelled() {
        // The ranged attacks of the trial and of the time-limit mode are a wind-up that spawns
        // the zone, then a plain active phase and a recovery: none holds its token past it.
        for engine in [
            worst_case::warmed_up(),
            crate::bench_fixtures::time_limit_case::warmed_up(),
        ] {
            let Run::V2 { world, .. } = engine.run() else {
                panic!("the live rules");
            };
            let mut zone_attacks = 0;
            for a in &world.attacks {
                let spawns_zone = a.phases.first().is_some_and(|p| !p.spawn.is_empty());
                if spawns_zone && a.phases.iter().skip(1).all(|p| p.hit_box.is_none()) {
                    zone_attacks += 1;
                    assert!(!a.channelled);
                }
            }
            assert!(zone_attacks > 0);
        }
    }

    #[test]
    fn the_bench_worlds_name_no_gauge() {
        // The worst case and the time-limit case are the shipped content: no elite or boss names a
        // `stagger_max`, so the stagger rules never run there.
        for engine in [
            worst_case::warmed_up(),
            crate::bench_fixtures::time_limit_case::warmed_up(),
        ] {
            let Run::V2 { world, .. } = engine.run() else {
                panic!("the live rules");
            };
            assert!(!world.has_stagger);
        }
    }

    // --- The knockdown ------------------------------------------------------------------------

    fn me() -> PlayerIdx {
        PlayerIdx::new(0).unwrap()
    }

    /// The world naming a knockdown of 30 ticks and a state whose enemy 0 can be knocked down.
    fn downable() -> (World, State) {
        let (mut w, mut s) = bare();
        w.tuning.down_ticks = 30;
        w.has_down = true;
        // No control list unless a test names one: the bundled list stays out.
        w.tuning.control_repeat_len = 0;
        w.tuning.control_reset_ticks = 0;
        w.tuning.control_immune_ticks = 0;
        let e = &mut s.enemies[0];
        assert!(e.elite.is_none() && e.hp > 0, "a regular enemy");
        e.stun_immune = 0;
        e.control = Default::default();
        (w, s)
    }

    fn applied(events: &[DomainEvent]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::StatusApplied {
                    status,
                    duration_ticks,
                    ..
                } if status.as_str() == "knocked_down" => Some(*duration_ticks),
                _ => None,
            })
            .collect()
    }

    fn list(w: &mut World) {
        w.tuning.control_repeat_len = 2;
        w.tuning.control_repeat_pcts[0] = 100;
        w.tuning.control_repeat_pcts[1] = 50;
        w.tuning.control_reset_ticks = 600;
        w.tuning.control_immune_ticks = 100;
    }

    #[test]
    fn a_knockdown_holds_the_enemy_for_the_rules_ticks_and_reports_it() {
        let (w, mut s) = downable();
        let mut events = Vec::new();
        knock_down(&w, &mut s, me(), 0, &mut events);
        assert_eq!(s.enemies[0].control.knocked_down, 30);
        assert_eq!(applied(&events), [30]);
        // A downed player's hit knocks nothing down.
        let (w, mut s) = downable();
        s.players[me()].hp = 0;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert_eq!(s.enemies[0].control.knocked_down, 0);
        // A stun-immune enemy is refused.
        let (w, mut s) = downable();
        s.enemies[0].stun_immune = 5;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert_eq!(s.enemies[0].control.knocked_down, 0);
    }

    #[test]
    fn a_knockdown_counts_down_and_its_end_starts_the_stun_immunity_when_no_list_is_named() {
        let (w, mut s) = downable();
        s.stun.immune_ticks = 100;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        let mut events = Vec::new();
        for _ in 0..29 {
            down_tick(&w, &mut s, &mut events);
        }
        assert_eq!(s.enemies[0].control.knocked_down, 1);
        assert!(events.is_empty());
        down_tick(&w, &mut s, &mut events);
        assert_eq!(s.enemies[0].control.knocked_down, 0);
        let id = s.enemies[0].id;
        assert_eq!(events, [DomainEvent::KnockdownEnded { target: id }]);
        assert_eq!(s.enemies[0].stun_immune, 100);
        // With a list the list's immunity governs: the end starts none.
        let (mut w, mut s) = downable();
        list(&mut w);
        s.stun.immune_ticks = 100;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        for _ in 0..30 {
            down_tick(&w, &mut s, &mut Vec::new());
        }
        assert_eq!(s.enemies[0].stun_immune, 0);
    }

    #[test]
    fn a_second_knockdown_counts_nothing_and_keeps_the_longer_remainder() {
        let (mut w, mut s) = downable();
        list(&mut w);
        s.enemies[0].control.knocked_down = 5;
        s.enemies[0].control.hard_controls = 1;
        let mut events = Vec::new();
        knock_down(&w, &mut s, me(), 0, &mut events);
        assert_eq!(s.enemies[0].control.hard_controls, 1, "counts nothing");
        assert_eq!(s.enemies[0].control.knocked_down, 30);
        assert_eq!(applied(&events), [30], "reported again");
        // A shorter one keeps the remainder and reports nothing: 30 * 50 / 100 = 15 < 25.
        let (mut w, mut s) = downable();
        list(&mut w);
        s.enemies[0].control.knocked_down = 25;
        s.enemies[0].control.hard_controls = 2;
        let mut events = Vec::new();
        knock_down(&w, &mut s, me(), 0, &mut events);
        assert_eq!(s.enemies[0].control.knocked_down, 25);
        assert!(applied(&events).is_empty());
    }

    #[test]
    fn a_knockdown_ends_a_flinch_and_shares_the_stun_s_list_and_count() {
        let (mut w, mut s) = downable();
        w.tuning.flinch_immune_ticks = 40;
        s.enemies[0].control.flinch = 7;
        let mut events = Vec::new();
        knock_down(&w, &mut s, me(), 0, &mut events);
        let id = s.enemies[0].id;
        assert_eq!(s.enemies[0].control.flinch, 0);
        assert_eq!(s.enemies[0].control.flinch_immune, 40);
        assert!(events.contains(&DomainEvent::FlinchEnded { target: id }));
        // A stun and a knockdown share the count: stun first, the knockdown keeps 15.
        let (mut w, mut s) = downable();
        list(&mut w);
        stun_owner_for(&w, &mut s);
        assert_eq!(s.enemies[0].control.hard_controls, 1);
        s.enemies[0].stun_immune = 0;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert_eq!(s.enemies[0].control.knocked_down, 15);
        // Past the list a knockdown is refused and the immunity starts.
        s.enemies[0].control.knocked_down = 0;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert_eq!(s.enemies[0].control.knocked_down, 0);
        assert_eq!(s.enemies[0].stun_immune, 100);
        // A stun and a knockdown on one enemy: each ends on its own.
        let (w, mut s) = downable();
        let source = s.players.id(me());
        s.enemies[0].stun_immune = 0;
        apply(&w, &mut s, Who::Enemy(0), source, 10, &mut Vec::new());
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert!(stunned(&s.enemies[0].statuses));
        assert_eq!(s.enemies[0].control.knocked_down, 30);
    }

    fn stun_owner_for(w: &World, s: &mut State) {
        let source = s.players.id(me());
        apply(w, s, Who::Enemy(0), source, 30, &mut Vec::new());
    }

    #[test]
    fn a_knockdown_frees_a_zone_leaving_runs_token_and_cools_a_channelled_one() {
        let (mut w, mut s, _) = ranged_run(zone_leaving());
        w.tuning.down_ticks = 30;
        w.has_down = true;
        s.enemies[0].stun_immune = 0;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert!(s.runs.is_empty(), "the run is gone");
        assert!(s.ranged_cooling.is_empty(), "{:?}", s.ranged_cooling);

        let (mut w, mut s, pool) = ranged_run(channelled());
        w.tuning.down_ticks = 30;
        w.has_down = true;
        s.enemies[0].stun_immune = 0;
        let until = s.tick + w.tuning.ranged_token_cooldown_ticks;
        knock_down(&w, &mut s, me(), 0, &mut Vec::new());
        assert!(s.runs.is_empty(), "the run is gone");
        assert_eq!(s.ranged_cooling.as_slice(), [(until, pool)]);
    }

    #[test]
    fn a_downed_enemy_is_given_no_ranged_token() {
        let (w, mut s) = downable();
        let up = super::super::select::Up::of(&s.players);
        let granted = |w: &World, s: &mut State| {
            super::attacks::grant_ranged(w, s, &up);
            s.scratch
                .ranged_grants
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
        };
        let standing = granted(&w, &mut s);
        let id = *standing
            .first()
            .expect("a ready ranged enemy in the fixture");
        let i = s.enemies.iter().position(|e| e.id == id).unwrap();
        s.enemies[i].control.knocked_down = 5;
        assert!(!granted(&w, &mut s).contains(&id));
    }

    #[test]
    fn knockdowns_never_pass_the_decode_caps() {
        use crate::limits::{MAX_CONTROL_REPEATS, MAX_DOWN_TICKS};
        let (mut w, mut s) = downable();
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for round in 0..2000u32 {
            let r = next();
            w.tuning.down_ticks = u16::try_from(1 + r % u64::from(MAX_DOWN_TICKS)).unwrap();
            if round % 3 == 0 {
                list(&mut w);
            } else {
                w.tuning.control_repeat_len = 0;
            }
            s.enemies[0].stun_immune = 0;
            match next() % 3 {
                0 => knock_down(&w, &mut s, me(), 0, &mut Vec::new()),
                1 => down_tick(&w, &mut s, &mut Vec::new()),
                _ => {
                    for _ in 0..(next() % 40) {
                        down_tick(&w, &mut s, &mut Vec::new());
                    }
                }
            }
            let c = &s.enemies[0].control;
            assert!(c.knocked_down <= MAX_DOWN_TICKS, "round {round}");
            assert!(c.hard_controls <= MAX_CONTROL_REPEATS, "round {round}");
        }
    }
}
