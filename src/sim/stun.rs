//! Stun: what a smash's stun does to the enemy it hits. The status is `add_stun`'s
//! (`entity.rs`). Only an enemy takes a stun: a boss and its parts resist it, and the main
//! character's and the companions' hits never stun anything. An elite takes its share of the
//! ticks, and a stun of no ticks does nothing.

use super::World;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::sim::state::{State, StatusKind, StatusState};
use crate::validate::limits::MAX_STUN_TICKS;

use super::attacks;
use super::entity::Who;

/// Whether `statuses` holds a stun.
pub(super) fn stunned(statuses: &[StatusState]) -> bool {
    statuses.iter().any(|st| st.kind == StatusKind::Stun)
}

/// Stuns `target` for a smash's `ticks`: all of them on a regular enemy, the elite's share on an
/// elite, nothing on any other target.
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
    if ticks > 0 {
        // The stun cancels what the enemy was doing: its running attack, in any phase. Orbs it
        // has spawned fly on.
        let id = s.id_of(target);
        s.add_stun(target, source, ticks, events);
        attacks::end_runs_of(stage, s, id);
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
