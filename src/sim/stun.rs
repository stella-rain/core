//! Stun: what a smash's stun does to the enemy it hits. The status is `add_stun`'s
//! (`entity.rs`). Only an enemy takes a stun: a boss and its parts resist it, and the main
//! character's and the companions' hits never stun anything. An elite takes its share of the
//! ticks, and a stun of no ticks does nothing.

use super::World;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::sim::state::{AgentState, State, StatusKind, StatusState};
use crate::sim::world::Tuning;
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
}
