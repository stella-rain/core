//! The worst combat tick (ADR-SIM-5, ADR-SIM-6): the crowd, zones, orbs and elites of
//! `worst_case`, with the three bounded queues of the combat rules at their caps on top: the
//! step repeats of Judgement Cut (`MAX_REPEATS_QUEUED` entries of `MAX_REPEAT_TARGETS` ids, all
//! due on the measured tick), every elite staggered with its breaks past the rules' refill list,
//! and every enemy at its hard-control count with the reset running. Shared by
//! `benches/worst_tick_combat.rs`, which counts its instructions, and the tests below and in
//! `tests/worst_tick_combat.rs`, which check that it reaches every cap it names.
//!
//! The weapon is the bundled `giant_sword` with the style `judgement_cut` taken, so the world
//! holds the style's repeat kinds (`World::has_repeat`) and the run reserves the queue. A press
//! of a step cannot reach 64 queued repeats (only a smash step of that style queues one), nor
//! can play stagger every elite or hold every enemy at its control cap on one tick, so the
//! fixture writes the three states into the run before each tick from `CAPS_FROM`
//! (`prepare`, through the crate-internal `Engine::run_mut`). The sword's steps stay out of the
//! crowd's reach, as the main character stands where the enemies' attacks cannot reach it.

use super::{common, worst_case};
use crate::attack::{HitBox, PhaseKind};
use crate::engine::{Engine, Run};
use crate::id::{EntityId, PlayerIdx};
use crate::input::{EquipAction, Input};
use crate::inline::InlineVec;
use crate::limits::{MAX_REPEAT_TARGETS, MAX_REPEATS_QUEUED, MAX_STAGGER_BREAKS};
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::sim::AttackIndex;
use crate::sim::state::{Repeat, RepeatAnchor, Stagger};
use crate::stage::Stage;
use serde_json::json;

/// Ticks to run before the tick that is measured (a placeholder, as `worst_case::WARMUP_TICKS`:
/// every enemy has arrived and the statuses stand at their cap long before it).
pub const WARMUP_TICKS: u32 = 420;

/// The first tick `prepare` runs before: the crowd and its elites are on the map from about
/// tick 210 (`worst_case::WAVES_AT`).
pub const CAPS_FROM: u32 = 300;

/// The weapon and its style: Judgement Cut names three `repeat` kinds (`content/expertise.json`).
const WEAPON: &str = "giant_sword";
const STYLE: &str = "judgement_cut";

/// The elites' `stagger_max` (the bundled elites' own, 400).
const ELITE_STAGGER_MAX: u32 = 400;

/// Expertise points given before the first tick, enough for every level of the weapon, so the
/// style point is held on tick 2 (`step`).
const EXPERTISE_POINTS: u32 = u32::MAX / 2;

/// `worst_case::stage` with the elites naming a stagger gauge and the bundled `giant_sword`.
pub fn stage() -> Stage {
    common::armed(
        worst_case::stage_with(Some(ELITE_STAGGER_MAX)),
        json!({ "preset": { "id": WEAPON } }),
    )
}

/// The run's content: that of `worst_case` and the bundled weapons and expertise.
pub fn content() -> RunContent {
    let bundled = RunContent::bundled();
    worst_case::content()
        .with_weapons(bundled.weapons().clone())
        .with_expertise(bundled.expertise().clone())
}

/// An engine at the start of the run: the equipment of `worst_case::start` on the sword, with
/// the weapon's expertise points seeded.
pub fn start() -> Engine {
    let stage = stage();
    let setup = RunSetup {
        seed: u64::from(stage.seed),
        weapon: WEAPON.into(),
        unlocked_styles: Some(vec![STYLE.into()]),
        ..RunSetup::default()
    };
    let mut engine = worst_case::start_in(&stage, &content(), &setup);
    engine
        .script_expertise(PlayerIdx::SOLO, EXPERTISE_POINTS)
        .expect("the stage gives expertise points");
    engine
}

/// The input of tick `tick` (from 1): that of `worst_case`, without aim assist and auto mode
/// (the sword has no assist range to widen and the main character stays where it stands).
pub fn input(tick: u32) -> Input {
    Input {
        assist: false,
        auto: false,
        ..worst_case::input(tick)
    }
}

/// Plays tick `tick` (the engine's next): the caps written first, from `CAPS_FROM`; after the
/// first step the weapon level has risen, and the style point is taken.
pub fn step(engine: &mut Engine, tick: u32) {
    if tick >= CAPS_FROM {
        prepare(engine);
    }
    engine.step(&[input(tick), Default::default(), Default::default()]);
    if tick == 1 {
        engine
            .act(PlayerIdx::SOLO, EquipAction::TreePick { option: 0 })
            .expect("the only unlocked style is taken");
    }
}

/// Writes the combat caps into the run, for the tick the engine plays next.
///
/// Repeats: the queue holds `MAX_REPEATS_QUEUED` entries, each a hit list of `MAX_REPEAT_TARGETS`
/// living enemies, due on that tick, spread round the crowd; each deals 1 damage, so the crowd
/// outlives the scan. Stagger: every elite is staggered, its gauge part full and its breaks
/// at the cap of the rules' refill list. Control: every enemy holds `hard_controls` at the
/// rules' list length with its reset running, so the next control takes the refuse-and-immune
/// path.
pub(crate) fn prepare(engine: &mut Engine) {
    let due = engine.tick() + 1;
    let Run::V2 { world, state, .. } = engine.run_mut() else {
        panic!("the live rules");
    };
    let (attack, phase) = repeat_phase(world);
    let ids: Vec<EntityId> = state
        .enemies
        .iter()
        .filter(|e| e.hp > 0)
        .map(|e| e.id)
        .collect();
    state.repeats.clear();
    if !ids.is_empty() {
        for n in 0..MAX_REPEATS_QUEUED {
            let list: InlineVec<EntityId, MAX_REPEAT_TARGETS> = (0..MAX_REPEAT_TARGETS)
                .map(|k| ids[(n * MAX_REPEAT_TARGETS + k) % ids.len()])
                .collect();
            state.repeats.push(Repeat {
                owner: 0,
                due,
                percent: 50,
                proc_pct: None,
                anchor: RepeatAnchor::HitList(list),
                attack,
                phase,
                damage: 1,
                width_pct: 0,
            });
        }
    }
    let t = world.tuning;
    let breaks = u16::from(t.stagger_refill_len).min(MAX_STAGGER_BREAKS) + 1;
    for e in &mut state.enemies {
        e.control.hard_controls = t.control_repeat_len;
        e.control.hard_control_reset = t.control_reset_ticks;
        if let Some(elite) = e.elite.as_mut() {
            elite.stagger = Stagger {
                gauge: 0,
                breaks,
                drain: 0,
                staggered: t.staggered_ticks,
            };
        }
    }
}

/// The attack of the run's table and the index of its first active phase with a hit box: what an
/// entry reads its box and damage type from (a hit list never tests the box).
fn repeat_phase(world: &crate::sim::World) -> (AttackIndex, u8) {
    for (i, a) in world.attacks.iter().enumerate() {
        for (j, p) in a.phases.iter().enumerate() {
            if p.kind == PhaseKind::Active && matches!(p.hit_box, Some(HitBox::Circle { .. })) {
                let at = AttackIndex(u16::try_from(i).expect("an attack index"));
                return (at, u8::try_from(j).expect("a phase index"));
            }
        }
    }
    panic!("an attack with an active circle");
}

/// An engine `WARMUP_TICKS` ticks into the run with the caps written for the next tick: the
/// next `step` (tick `WARMUP_TICKS + 1`) is the measured one, and its input is `input`.
pub fn warmed_up() -> Engine {
    let mut engine = start();
    for tick in 1..=WARMUP_TICKS {
        step(&mut engine, tick);
    }
    prepare(&mut engine);
    engine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Load, Outcome};
    use crate::limits::MAX_ELITES_ALIVE;

    fn state_of(e: &Engine) -> (&crate::sim::World, &crate::sim::state::State) {
        match e.run() {
            Run::V2 { world, state, .. } => (world, state),
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    /// Tests-first 7: on the measured tick the queues and gauges stand at their caps.
    #[test]
    fn the_measured_tick_holds_the_combat_caps() {
        let engine = warmed_up();
        let (world, s) = state_of(&engine);
        assert!(world.has_repeat && world.has_stagger);
        let due = engine.tick() + 1;
        assert_eq!(s.repeats.len(), MAX_REPEATS_QUEUED);
        for r in &s.repeats {
            assert_eq!(r.due, due);
            let RepeatAnchor::HitList(ids) = r.anchor else {
                panic!("a hit list");
            };
            assert_eq!(ids.len(), MAX_REPEAT_TARGETS);
        }
        let elites: Vec<_> = s.enemies.iter().filter(|e| e.elite.is_some()).collect();
        assert_eq!(elites.len(), usize::from(MAX_ELITES_ALIVE));
        for e in &elites {
            let st = e.elite.as_ref().unwrap().stagger;
            assert!(st.staggered > 0 && st.breaks > u16::from(world.tuning.stagger_refill_len));
            assert!(world.defs.agent(e.def).stagger_max.is_some());
        }
        assert_eq!(world.tuning.control_repeat_len, 3);
        for e in s.enemies.iter().filter(|e| e.elite.is_none()) {
            assert_eq!(e.control.hard_controls, 3);
            assert!(e.control.hard_control_reset > 0);
        }
        assert_eq!(s.zones.len(), 128);
        assert_eq!(s.orbs.len(), 32);
        assert_eq!(s.enemies.len(), usize::from(super::super::enemy_cap()));
    }

    /// The measured tick deals every queued repeat, and the next tick's `prepare` refills it.
    #[test]
    fn the_measured_tick_deals_the_queue() {
        let mut engine = warmed_up();
        engine.step(&[input(WARMUP_TICKS + 1), Default::default(), Default::default()]);
        assert_eq!(engine.outcome(), Outcome::Running);
        let (_, s) = state_of(&engine);
        assert!(s.repeats.is_empty(), "every entry was due on the tick");
    }

    /// The weight `Load` sees of a tick.
    fn weight(load: Load) -> u64 {
        [load.enemies, load.tasks, load.zones, load.orbs, load.emitter_steps]
            .iter()
            .map(|&n| u64::from(n))
            .sum()
    }

    /// Tests-first 9: no tick of the 60 after the warm-up outweighs the measured one, and the
    /// caps hold on each (a gap names `CAPS_FROM`). `Load` does not see the repeats, so a tie is
    /// settled by counting instructions (the bench).
    #[test]
    fn the_measured_tick_is_not_outweighed_and_the_caps_hold_tick_after_tick() {
        let mut engine = warmed_up();
        let mut weights = Vec::new();
        for tick in WARMUP_TICKS + 1..=WARMUP_TICKS + 60 {
            let (_, s) = state_of(&engine);
            assert_eq!(s.repeats.len(), MAX_REPEATS_QUEUED, "tick {tick}");
            step(&mut engine, tick);
            assert_eq!(engine.outcome(), Outcome::Running, "tick {tick}");
            let load = engine.load();
            assert_eq!(load.zones, 128, "tick {tick}");
            assert_eq!(load.orbs, 32, "tick {tick}");
            assert_eq!(load.enemies, u32::from(super::super::enemy_cap()), "tick {tick}");
            weights.push((tick, weight(load)));
            prepare(&mut engine);
        }
        let top = weights.iter().map(|&(_, w)| w).max().unwrap();
        let measured = weights[0].1;
        assert_eq!(
            measured, top,
            "the measured tick {} is outweighed (raise or move WARMUP_TICKS): {weights:?}",
            WARMUP_TICKS + 1
        );
    }
}
