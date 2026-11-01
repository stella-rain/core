//! `encode_state` and `decode_state`: the whole run state as savepoint bytes, and back.
//!
//! The bytes are the `State`, the `RunLedger` and the `Counted`, in that order, with no header
//! (the file's is the savepoint file's) and no trailing bytes. What the content and the setup
//! decide is not in them: `decode_state` takes it from a fresh engine of the same run, and
//! checks every length and the content indices it lists against it (`check_indices`), so a
//! stale file is refused, not indexed out of range on a later tick (ADR-SVC-1). It does not
//! check that the values are ones a run can reach (a boss phase past its last, a stat point
//! past its limit): that is for the savepoint file's hash and for the fuzz target on this
//! function to show whether a check is needed.

use crate::engine::{Engine, Run, Saved};
use crate::ledger::{Counted, RunLedger};
use crate::rewards::ORBIT_SET_BASE;
use crate::run_content::RunContent;

use super::{Decode, DecodeError, Encode, Reader, Writer};

/// The state, the ledger and the tallies of `engine` as bytes.
pub(crate) fn encode_state(engine: &Engine) -> Vec<u8> {
    let (ledger, counted) = engine.saved_parts();
    match engine.run() {
        Run::V1 { state, .. } => v1::encode(state, ledger, counted),
        Run::V2 { state, .. } => v2::encode(state, ledger, counted),
    }
}

/// The state, the ledger and the tallies in `bytes`, for a run of `fresh`'s content, read by the
/// rules module of `fresh`'s version. `fresh` is an engine started from the same setup and
/// content, untouched: everything the content and the setup decide comes from it, and every
/// length and index in the bytes is checked against it.
pub(crate) fn decode_state(
    bytes: &[u8],
    fresh: &Engine,
) -> Result<(Saved, RunLedger, Counted), DecodeError> {
    let (fresh_ledger, fresh_counted) = fresh.saved_parts();
    let content = fresh.content();
    match fresh.run() {
        Run::V1 { world, state } => {
            v1::decode(bytes, world, content, (state, fresh_ledger, fresh_counted))
                .map(|(s, l, c)| (Saved::V1(s), l, c))
        }
        Run::V2 { world, state } => {
            v2::decode(bytes, world, content, (state, fresh_ledger, fresh_counted))
                .map(|(s, l, c)| (Saved::V2(s), l, c))
        }
    }
}

/// The coding of one rules module's state: the same text for each version, which owns its types
/// (ADR-REPL-9). Frozen versions keep their text; a change to the live one is a new version's.
macro_rules! state_codec {
    ($sim:ident, $get:expr) => {
        use crate::$sim::World;
        use crate::$sim::state::State;

        use super::*;

        pub(super) fn encode(state: &State, ledger: &RunLedger, counted: &Counted) -> Vec<u8> {
            let mut w = Writer::new();
            state.encode(&mut w);
            ledger.encode(&mut w);
            counted.encode(&mut w);
            w.finish()
        }

        pub(super) fn decode(
            bytes: &[u8],
            world: &World,
            content: &RunContent,
            fresh: (&State, &RunLedger, &Counted),
        ) -> Result<(State, RunLedger, Counted), DecodeError> {
            let (fresh_state, fresh_ledger, fresh_counted) = fresh;
            let mut r = Reader::new(bytes);
            let mut state = State::decode_over(&mut r, fresh_state)?;
            let ledger = RunLedger::decode(&mut r)?;
            let counted = Counted::decode(&mut r)?;
            r.finish()?;

            check_shape(&state, &counted, fresh_state, fresh_counted)?;
            // The content-sized fields the bytes do not hold.
            if let (Some(p), Some(f)) = (state.progress.as_mut(), fresh_state.progress.as_ref()) {
                p.stats = f.stats.clone();
            }
            if let (Some(h), Some(f)) = (state.horde.as_mut(), fresh_state.horde.as_ref()) {
                h.boss_ticks = f.boss_ticks.clone();
                h.no_call_ticks = f.no_call_ticks.clone();
                h.mid_boss_hp_pct = f.mid_boss_hp_pct.clone();
            }
            check_indices(&state, world, content)?;

            // Only now are the runs known to index real attacks and phases.
            state.move_index.rebuild(world, &state.runs);
            crate::$sim::reserve(world, &mut state);
            let ledger = ledger.rebuilt_from(fresh_ledger)?;
            Ok((state, ledger, counted))
        }

        fn differs(what: &'static str) -> DecodeError {
            DecodeError::Invalid(what)
        }

        /// Each length the content decides, and each `Option` it decides, equals the fresh engine's.
        fn check_shape(
            s: &State,
            counted: &Counted,
            fresh: &State,
            fresh_counted: &Counted,
        ) -> Result<(), DecodeError> {
            fn same_presence<T>(
                a: &Option<T>,
                b: &Option<T>,
                what: &'static str,
            ) -> Result<(), DecodeError> {
                if a.is_some() == b.is_some() {
                    Ok(())
                } else {
                    Err(differs(what))
                }
            }
            if s.spawned.len() != fresh.spawned.len() {
                return Err(differs("spawned length differs from the content"));
            }
            same_presence(
                &s.progress,
                &fresh.progress,
                "progress presence differs from the content",
            )?;
            same_presence(
                &s.stars,
                &fresh.stars,
                "stars presence differs from the content",
            )?;
            same_presence(
                &s.heal_pickups,
                &fresh.heal_pickups,
                "heal_pickups presence differs from the content",
            )?;
            same_presence(
                &s.expertise,
                &fresh.expertise,
                "expertise presence differs from the content",
            )?;
            same_presence(
                &s.horde,
                &fresh.horde,
                "horde presence differs from the content",
            )?;
            same_presence(
                &s.stardust,
                &fresh.stardust,
                "stardust presence differs from the content",
            )?;
            if let (Some(p), Some(f)) = (&s.progress, &fresh.progress) {
                if p.spent.len() != f.stats.len() {
                    return Err(differs("spent length differs from the content"));
                }
                if p.priority.len() != f.stats.len() {
                    return Err(differs("priority length differs from the content"));
                }
            }
            if let (Some(a), Some(f)) = (&s.stars, &fresh.stars) {
                if a.pool.len() != f.pool.len() {
                    return Err(differs("star pool length differs from the content"));
                }
                if a.taken.len() != f.taken.len() {
                    return Err(differs("star taken length differs from the content"));
                }
                if a.banished.len() != f.banished.len() {
                    return Err(differs("star banished length differs from the content"));
                }
                if a.set_counts.len() != f.set_counts.len() {
                    return Err(differs("star set_counts length differs from the content"));
                }
            }
            if let (Some(a), Some(f)) = (&s.expertise, &fresh.expertise) {
                if a.taken.len() != f.taken.len() {
                    return Err(differs("expertise taken length differs from the content"));
                }
            }
            if let (Some(a), Some(f)) = (&s.horde, &fresh.horde) {
                if a.used.len() != f.used.len() {
                    return Err(differs("horde used length differs from the content"));
                }
                if a.held.len() != f.held.len() {
                    return Err(differs("horde held length differs from the content"));
                }
            }
            if counted.taken.len() != fresh_counted.taken.len() {
                return Err(differs("counted taken length differs from the content"));
            }
            if counted.tiers.len() != fresh_counted.tiers.len() {
                return Err(differs("counted tiers length differs from the content"));
            }
            // Set at the start from the content, and never changed: a different number is another run.
            if s.max_attack_speed_pct != fresh.max_attack_speed_pct {
                return Err(differs("max_attack_speed_pct differs from the content"));
            }
            if s.enemy_attack_damage != fresh.enemy_attack_damage {
                return Err(differs("enemy_attack_damage differs from the content"));
            }
            if s.max_rune_haste_pct != fresh.max_rune_haste_pct {
                return Err(differs("max_rune_haste_pct differs from the content"));
            }
            Ok(())
        }

        /// Every index the bytes hold is below the length of what it indexes in the fresh engine's run,
        /// which the `Decode` impls cannot see. Reads of these indices in the rules are unchecked
        /// (`world.attacks[..]`, `emitters[..]`, `defs.get`) or would silently skip a thing.
        pub(super) fn check_indices(
            s: &State,
            world: &World,
            content: &RunContent,
        ) -> Result<(), DecodeError> {
            let (defs, bosses, attacks, programs) = (
                world.defs.len(),
                world.bosses.len(),
                world.attacks.len(),
                world.emitters.len(),
            );
            let kinds = s.kinds.len();
            let rewards = content.rewards().rewards().len();
            let sets = content.rewards().constellations().sets().len();
            let skills = content.rewards().stage_skills().skills().len();
            let below = |n: usize, len: usize, what: &'static str| {
                if n < len {
                    Ok(())
                } else {
                    Err(DecodeError::Invalid(what))
                }
            };
            for a in s.companions.iter().chain(&s.enemies) {
                below(usize::from(a.def.0), defs, "agent def out of range")?;
                below(usize::from(a.kind.0), kinds, "agent kind out of range")?;
            }
            if let Some(b) = &s.boss {
                below(usize::from(b.boss.0), bosses, "boss index out of range")?;
                below(usize::from(b.kind.0), kinds, "boss kind out of range")?;
                for p in &b.parts {
                    below(usize::from(p.kind.0), kinds, "part kind out of range")?;
                }
            }
            for run in &s.runs {
                below(
                    usize::from(run.attack.0),
                    attacks,
                    "run attack out of range",
                )?;
                let phases = world.attacks[usize::from(run.attack.0)].phases.len();
                below(run.phase as usize, phases, "run phase out of range")?;
            }
            for t in &s.tasks {
                below(t.program.0 as usize, programs, "task program out of range")?;
            }
            for o in &s.orbs {
                below(o.program.0 as usize, programs, "orb program out of range")?;
                if let Some(h) = &o.hook {
                    below(h.program.0 as usize, programs, "hook program out of range")?;
                }
            }
            for z in &s.zones {
                below(z.program.0 as usize, programs, "zone program out of range")?;
            }
            // The one place the macro reads the player: the accessor each version passes.
            let player: fn(&State) -> &crate::$sim::state::PlayerState = $get;
            for o in &player(s).orbits {
                // The key of an orbit is its reward's place, or `ORBIT_SET_BASE` and a set's place.
                let in_range = match o.reward.checked_sub(ORBIT_SET_BASE) {
                    None => usize::from(o.reward) < rewards,
                    Some(set) => usize::from(set) < sets,
                };
                if !in_range {
                    return Err(DecodeError::Invalid("orbit reward out of range"));
                }
            }
            if let Some(slot) = &player(s).skill {
                below(usize::from(slot.index), skills, "skill index out of range")?;
            }
            if let Some(f) = &s.stars {
                for p in &f.pool {
                    below(usize::from(*p), rewards, "star pool entry out of range")?;
                }
                let offer = f.offer.iter().flat_map(|o| &o.candidates);
                for c in offer {
                    below(usize::from(*c), rewards, "offer candidate out of range")?;
                }
            }
            if let Some(g) = &s.expertise {
                if let Some(o) = &g.offer {
                    if o.kind == crate::snapshot::ExpertiseOfferKind::Common {
                        for c in &o.candidates {
                            below(
                                usize::from(*c),
                                g.taken.len(),
                                "expertise candidate out of range",
                            )?;
                        }
                    }
                }
            }
            Ok(())
        }
    };
}

mod v1 {
    state_codec!(sim_v1, |s| &s.player);
}

mod v2 {
    // shim: a run of one has its player at the first place.
    state_codec!(sim, |s| &s.players[crate::sim::state::PlayerIdx::SOLO]);
}

#[cfg(test)]
mod players_tests {
    //! The index check reads every player, not the first only (ADR-SIM-12).

    use crate::bench_fixtures::worst_case;
    use crate::engine::Run;
    use crate::id::PlayerIdx;
    use crate::sim::state::{OrbitState, Players};

    use super::v2::check_indices;

    #[test]
    fn the_savepoint_check_reads_every_player() {
        let e = worst_case::warmed_up();
        let Run::V2 { world, state } = e.run() else {
            panic!("the live rules")
        };
        let rewards = e.content().rewards().rewards().len();
        let bad = OrbitState {
            reward: u16::try_from(rewards).unwrap(),
            orbs: 1,
            round: 0,
            cooldown_left: 0,
            angle: 0,
        };
        let first = state.players[PlayerIdx::SOLO].clone();
        let build = |bad_on: usize| {
            let (mut a, mut b) = (first.clone(), first.clone());
            [&mut a, &mut b][bad_on].orbits.push(bad.clone());
            let mut s = state.clone();
            s.players = Players::of(vec![a, b]).unwrap();
            s
        };
        let refused = Err(crate::savepoint::codec::DecodeError::Invalid(
            "orbit reward out of range",
        ));
        assert_eq!(check_indices(&build(0), world, e.content()), refused);
        assert_eq!(check_indices(&build(1), world, e.content()), refused);
        let mut ok = state.clone();
        ok.players = Players::of(vec![first.clone(), first]).unwrap();
        assert_eq!(check_indices(&ok, world, e.content()), Ok(()));
    }
}
