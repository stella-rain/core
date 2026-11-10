//! `encode_state` and `decode_state`: the whole run state as savepoint bytes, and back.
//!
//! The bytes are the player stats record (`STATS_RECORD_LEN`, fixed length), then the `State`, the
//! `RunLedger` and the `Counted`, with no header (the file's is the savepoint file's) and no
//! trailing bytes. What the content and the setup
//! decide is not in them: `decode_state` takes it from a fresh engine of the same run, and
//! checks every length and the content indices it lists against it (`check_indices`), so a
//! stale file is refused, not indexed out of range on a later tick (ADR-SVC-1). It does not
//! check that the values are ones a run can reach (a boss phase past its last, a stat point
//! past its limit): that is for the savepoint file's hash and for the fuzz target on this
//! function to show whether a check is needed.

use crate::engine::{Engine, Run, Saved};
use crate::ledger::{Counted, PlayerStats, RunLedger, STATS_RECORD_LEN};
use crate::rewards::ORBIT_SET_BASE;
use crate::run_content::RunContent;

use super::{Decode, DecodeError, Encode, Reader, Writer};

/// The player stats, the state, the ledger and the tallies of `engine` as bytes. The record
/// comes first and has a fixed length, so `decode_state` takes it off before the version
/// module's text reads the rest, and the frozen text of `state_codec!` is untouched.
pub(crate) fn encode_state(engine: &Engine) -> Vec<u8> {
    let (ledger, counted, stats) = engine.saved_parts();
    let mut w = Writer::new();
    stats.encode(&mut w);
    let mut bytes = w.finish();
    bytes.extend(match engine.run() {
        Run::V1 { state, .. } => v1::encode(state, ledger, counted),
        Run::V2 { state, .. } => v2::encode(state, ledger, counted),
    });
    bytes
}

/// The player stats, state, ledger and tallies in `bytes`, for a run of `fresh`'s content, read
/// by the rules module of `fresh`'s version. `fresh` is an engine started from the same setup
/// and content, untouched: everything the content and the setup decide comes from it, and every
/// length and index in the bytes is checked against it. The stats record is trusted like the
/// replay prefix (ADR-SAVE-4), but a slot outside the party must be zero.
pub(crate) fn decode_state(
    bytes: &[u8],
    fresh: &Engine,
) -> Result<(Saved, RunLedger, Counted, PlayerStats), DecodeError> {
    let (fresh_ledger, fresh_counted, _) = fresh.saved_parts();
    let content = fresh.content();
    let (record, rest) = bytes
        .split_at_checked(STATS_RECORD_LEN)
        .ok_or(DecodeError::Truncated)?;
    // A run of version 1 has the party of one.
    let mask = match fresh.run() {
        Run::V1 { .. } => 0b001,
        Run::V2 { state, .. } => state.players.mask(),
    };
    let stats = PlayerStats::decode_for(&mut Reader::new(record), mask)?;
    match fresh.run() {
        Run::V1 { world, state, .. } => {
            v1::decode(rest, world, content, (state, fresh_ledger, fresh_counted))
                .map(|(s, l, c)| (Saved::V1(s), l, c, stats))
        }
        Run::V2 { world, state } => {
            v2::decode(rest, world, content, (state, fresh_ledger, fresh_counted))
                .map(|(s, l, c)| (Saved::V2(s), l, c, stats))
        }
    }
}

/// The coding of one rules module's state: the same text for each version, which owns its types
/// (ADR-REPL-2). Frozen versions keep their text; a change to the live one is a new version's.
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
            // The one place the macro reads the players: the accessor each version passes.
            let players: fn(&State) -> Vec<&crate::$sim::state::PlayerState> = $get;
            for p in players(s) {
                for o in &p.orbits {
                    // The key of an orbit is its reward's place, or `ORBIT_SET_BASE` and a set's place.
                    let in_range = match o.reward.checked_sub(ORBIT_SET_BASE) {
                        None => usize::from(o.reward) < rewards,
                        Some(set) => usize::from(set) < sets,
                    };
                    if !in_range {
                        return Err(DecodeError::Invalid("orbit reward out of range"));
                    }
                }
                if let Some(slot) = &p.skill {
                    below(usize::from(slot.index), skills, "skill index out of range")?;
                }
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
                    if o.kind == crate::$sim::views::ExpertiseOfferKind::Common {
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
    state_codec!(sim_v1, |s| vec![&s.player]);
}

/// The live module's copy of `state_codec!`: it follows the live `sim` (ADR-REPL-2) while the macro
/// above keeps the frozen text. A fix to a check shared by both is made in both until the frozen
/// version is retired.
macro_rules! state_codec_v2 {
    ($sim:ident, $get:expr) => {
        use crate::$sim::World;
        use crate::$sim::state::State;

        use super::*;
        use crate::id::PlayerIdx;
        // Format 1 holds one player: the progress below is slot 0's career.

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
            for me in state.players.indices().collect::<Vec<_>>() {
                if let (Some(p), Some(f)) = (
                    state.players.career_mut(me).progress.as_mut(),
                    fresh_state
                        .players
                        .career(PlayerIdx::SOLO)
                        .progress
                        .as_ref(),
                ) {
                    p.stats = f.stats.clone();
                }
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
        pub(super) fn check_shape(
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
                &s.horde,
                &fresh.horde,
                "horde presence differs from the content",
            )?;
            if let (Some(a), Some(f)) = (&s.stars, &fresh.stars) {
                if a.pool.len() != f.pool.len() {
                    return Err(differs("star pool length differs from the content"));
                }
            }
            for me in s.players.indices() {
                same_presence(
                    &s.players.career(me).progress,
                    &fresh.players.career(PlayerIdx::SOLO).progress,
                    "progress presence differs from the content",
                )?;
                same_presence(
                    &s.players.career(me).expertise,
                    &fresh.players.career(PlayerIdx::SOLO).expertise,
                    "expertise presence differs from the content",
                )?;
                same_presence(
                    &s.players.career(me).stardust,
                    &fresh.players.career(PlayerIdx::SOLO).stardust,
                    "stardust presence differs from the content",
                )?;
                same_presence(
                    &s.players.career(me).equipment.lumine,
                    &fresh.players.career(PlayerIdx::SOLO).equipment.lumine,
                    "lumine presence differs from the setup",
                )?;
                // Equipment mode: the slots are the fresh engine's layout, each holds the kind
                // of item it takes, and the runes the Main fires are the rune slots filled.
                let fresh_slots = &fresh.players.career(PlayerIdx::SOLO).equipment.slots;
                let career = s.players.career(me);
                if !crate::sim::same_layout(&career.equipment.slots, fresh_slots) {
                    return Err(differs("equipment slots differ from the layout"));
                }
                if !crate::sim::holds_right_kinds(&career.equipment) {
                    return Err(differs("equipment slot holds the wrong kind"));
                }
                if !fresh_slots.is_empty()
                    && !crate::sim::runes_match_slots(&career.equipment, &s.players[me].runes)
                {
                    return Err(differs("runes differ from the rune slots"));
                }
                if let (Some(p), Some(f)) = (
                    &s.players.career(me).progress,
                    &fresh.players.career(PlayerIdx::SOLO).progress,
                ) {
                    if p.spent.len() != f.stats.len() {
                        return Err(differs("spent length differs from the content"));
                    }
                    if p.priority.len() != f.stats.len() {
                        return Err(differs("priority length differs from the content"));
                    }
                }
                // Slot 0's offers: the content-sized lists are the slot's.
                if let (Some(a), Some(f)) = (
                    &s.players.career(me).offers,
                    &fresh.players.career(PlayerIdx::SOLO).offers,
                ) {
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
                if let (Some(a), Some(f)) = (
                    &s.players.career(me).expertise,
                    &fresh.players.career(PlayerIdx::SOLO).expertise,
                ) {
                    if a.taken.len() != f.taken.len() {
                        return Err(differs("expertise taken length differs from the content"));
                    }
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
            // The one place the macro reads the players: the accessor each version passes.
            let players: fn(&State) -> Vec<&crate::$sim::state::PlayerState> = $get;
            for p in players(s) {
                for o in &p.orbits {
                    // The key of an orbit is its reward's place, or `ORBIT_SET_BASE` and a set's place.
                    let in_range = match o.reward.checked_sub(ORBIT_SET_BASE) {
                        None => usize::from(o.reward) < rewards,
                        Some(set) => usize::from(set) < sets,
                    };
                    if !in_range {
                        return Err(DecodeError::Invalid("orbit reward out of range"));
                    }
                }
                if let Some(slot) = &p.skill {
                    below(usize::from(slot.index), skills, "skill index out of range")?;
                }
            }
            if let Some(f) = &s.stars {
                for p in &f.pool {
                    below(usize::from(*p), rewards, "star pool entry out of range")?;
                }
            }
            for me in s.players.indices() {
                if let Some(o) = &s.players.career(me).offers {
                    let offer = o.offer.iter().flat_map(|o| &o.candidates);
                    for c in offer {
                        below(usize::from(*c), rewards, "offer candidate out of range")?;
                    }
                }
                // A star held in the inventory, a slot or the Lumine slot is a place in the
                // rewards file, and not the random rune (it is never a drop).
                let equipment = &s.players.career(me).equipment;
                let held = equipment
                    .inventory
                    .iter()
                    .chain(equipment.slots.iter().filter_map(|s| s.held.as_ref()))
                    .chain(equipment.lumine.iter().filter_map(|s| s.held.as_ref()));
                for h in held {
                    if let crate::sim::equipment::Held::Star(p) = h {
                        below(usize::from(*p), rewards, "equipment star out of range")?;
                        if content.rewards().rewards()[usize::from(*p)].is_random_rune() {
                            return Err(DecodeError::Invalid("equipment star is the random rune"));
                        }
                    }
                }
                // Equipment mode: a star in a category slot is of that category, and no two
                // equipped stars share an option (the rules of `slots::check`).
                if world.equipment_mode {
                    match crate::sim::misfit(&world.equipment_rules, content.rewards(), equipment) {
                        Some(crate::sim::Misfit::Category) => {
                            return Err(DecodeError::Invalid(
                                "equipment star does not fit its slot's category",
                            ));
                        }
                        Some(crate::sim::Misfit::SharedOption) => {
                            return Err(DecodeError::Invalid("equipment stars share an option"));
                        }
                        None => {}
                    }
                }
                if let Some(g) = &s.players.career(me).expertise {
                    crate::sim::check_tree(world, content.expertise(), g)
                        .map_err(DecodeError::Invalid)?;
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
            }
            Ok(())
        }
    };
}

mod v2 {
    state_codec_v2!(sim, |s| s.players.iter().collect());
}

#[cfg(test)]
mod players_tests {
    //! The index check reads every player, not the first only (ADR-SIM-7).

    use crate::bench_fixtures::worst_case;
    use crate::engine::{Engine, Run};
    use crate::id::PlayerIdx;
    use crate::sim::state::{OrbitState, Players};

    use super::v2::{check_indices, check_shape};

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
            [&mut a, &mut b][bad_on].orbits.push(bad);
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
    /// T23: a gauge that took more of the weapon tree than its level earned, or a place past the
    /// style's choices, is refused for every present slot; a legal gauge decodes.
    #[test]
    fn the_savepoint_refuses_a_forged_tree() {
        use crate::run_setup::{BuildMode, RunSetup};
        use crate::savepoint::DecodeError;

        let e = Engine::start_party(
            &RunSetup {
                mode: "time_limit".into(),
                build_mode: BuildMode::Equipment,
                weapon: "giant_hammer".into(),
                ..RunSetup::default()
            },
            2,
        )
        .unwrap();
        let Run::V2 { world, state } = e.run() else {
            panic!("the live rules")
        };
        let gauge = |me: u8, level: u32, style: Option<u8>, path: &[u8]| {
            let mut s = state.clone();
            let g = s
                .players
                .career_mut(PlayerIdx::new(me).unwrap())
                .expertise
                .as_mut()
                .expect("a gauge");
            (g.level, g.style, g.path) = (level, style, path.to_vec());
            s
        };
        let taken = Err(DecodeError::Invalid("expertise tree taken past the level"));
        let range = Err(DecodeError::Invalid("expertise tree place out of range"));
        for me in 0..2 {
            let check = |s| check_indices(&s, world, e.content());
            assert_eq!(
                check(gauge(me, 2, Some(0), &[])),
                taken,
                "style before its level"
            );
            assert_eq!(
                check(gauge(me, 3, Some(0), &[0])),
                taken,
                "path past the level"
            );
            assert_eq!(
                check(gauge(me, 9, None, &[0])),
                taken,
                "path without a style"
            );
            assert_eq!(
                check(gauge(me, 3, Some(3), &[])),
                range,
                "style past the styles"
            );
            assert_eq!(
                check(gauge(me, 6, Some(0), &[1])),
                range,
                "place past the choices"
            );
            assert_eq!(check(gauge(me, 3, Some(2), &[])), Ok(()));
            assert_eq!(check(gauge(me, 6, Some(1), &[0])), Ok(()));
            assert_eq!(check(gauge(me, 0, None, &[])), Ok(()));
        }
    }

    /// T14: an equipment star out of range, or the random rune, is refused for every present
    /// slot, wherever it is held.
    #[test]
    fn the_savepoint_refuses_a_bad_equipment_index() {
        use crate::savepoint::DecodeError;
        use crate::sim::equipment::{EquipSlot, Held, SlotKind};

        let e = crate::bench_fixtures::offers_cases::worst_warmed_up().into_party(2);
        let Run::V2 { world, state } = e.run() else {
            panic!("the live rules")
        };
        let rewards = e.content().rewards().rewards();
        let random_rune = rewards
            .iter()
            .position(|r| r.is_random_rune())
            .map(|i| u16::try_from(i).unwrap())
            .expect("the bundled file has the random rune");
        let len = u16::try_from(rewards.len()).unwrap();
        let ordinary =
            u16::try_from(rewards.iter().position(|r| !r.is_random_rune()).unwrap()).unwrap();
        let put = |me: u8, place: usize, held: Held| {
            let mut s = state.clone();
            let slot = EquipSlot {
                kind: SlotKind::Category(0),
                held: Some(held),
                level: 1,
                expertise: 0,
            };
            let eq = &mut s.players.career_mut(PlayerIdx::new(me).unwrap()).equipment;
            match place {
                0 => eq.inventory.push(held),
                1 => eq.slots.push(slot),
                _ => {
                    eq.lumine = Some(EquipSlot {
                        kind: SlotKind::Lumine,
                        ..slot
                    })
                }
            }
            s
        };
        for me in 0..2 {
            for place in 0..3 {
                for (held, message) in [
                    (Held::Star(len), "equipment star out of range"),
                    (Held::Star(random_rune), "equipment star is the random rune"),
                ] {
                    assert_eq!(
                        check_indices(&put(me, place, held), world, e.content()),
                        Err(DecodeError::Invalid(message)),
                        "slot {me} place {place}"
                    );
                }
                let fine = put(me, place, Held::Star(ordinary));
                assert_eq!(check_indices(&fine, world, e.content()), Ok(()));
                let rune = put(me, place, Held::Rune(crate::rune::Rune::Thunder));
                assert_eq!(check_indices(&rune, world, e.content()), Ok(()));
            }
        }
    }

    #[test]
    fn a_lumine_slot_the_setup_does_not_name_is_refused() {
        use crate::savepoint::DecodeError;
        use crate::sim::equipment::{EquipSlot, Held, SlotKind};

        let e = worst_case::warmed_up();
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let counted = e.saved_parts().1;
        assert_eq!(check_shape(state, counted, state, counted), Ok(()));
        let mut bad = state.clone();
        bad.players.career_mut(PlayerIdx::SOLO).equipment.lumine = Some(EquipSlot {
            kind: SlotKind::Lumine,
            held: Some(Held::Star(0)),
            level: 1,
            expertise: 0,
        });
        assert_eq!(
            check_shape(&bad, counted, state, counted),
            Err(DecodeError::Invalid(
                "lumine presence differs from the setup"
            ))
        );
    }

    #[test]
    fn slot_0s_career_round_trips_and_its_presence_is_checked() {
        use crate::bench_fixtures::{offers_cases, time_limit_case};
        use crate::savepoint::DecodeError;
        use crate::savepoint::state::{decode_state, encode_state};

        let e = offers_cases::time_limit_tick().engine;
        let fresh = Engine::new_with(&time_limit_case::stage(), e.content()).unwrap();
        let (
            Run::V2 { state: before, .. },
            Run::V2 {
                state: blank,
                world,
            },
        ) = (e.run(), fresh.run())
        else {
            panic!("the live rules")
        };
        let had = before.players.career(PlayerIdx::SOLO).progress.clone();
        assert!(had.is_some(), "the case has levels");
        let (saved, ledger, counted, stats) = decode_state(&encode_state(&e), &fresh).unwrap();
        let back = fresh.clone().with_saved(saved, ledger, counted, stats);
        let Run::V2 { state: after, .. } = back.run() else {
            panic!("the live rules")
        };
        let mut want = had.unwrap();
        want.stats = blank
            .players
            .career(PlayerIdx::SOLO)
            .progress
            .as_ref()
            .unwrap()
            .stats
            .clone();
        assert_eq!(after.players.career(PlayerIdx::SOLO).progress, Some(want));
        assert_eq!(back.state_hash(), e.state_hash());

        let _ = world;
        let mut bad = before.clone();
        bad.players.career_mut(PlayerIdx::SOLO).progress = None;
        let counted = e.saved_parts().1;
        assert_eq!(
            check_shape(&bad, counted, blank, counted),
            Err(DecodeError::Invalid(
                "progress presence differs from the content"
            ))
        );
    }
}

#[cfg(test)]
mod slots_tests {
    //! The equipment slots of every present player are the layout, hold the right kind of item,
    //! and the runes the Main fires are the rune slots filled (ADR-SIM-3 point 2, ADR-SIM-7).

    use crate::engine::{Engine, Run};
    use crate::run_setup::{BuildMode, RunSetup};
    use crate::rune::Rune;
    use crate::savepoint::DecodeError;
    use crate::sim::equipment::Held;
    use crate::sim::state::{HeldRune, State};

    use super::decode_state;
    use super::v2::{check_indices, check_shape};

    fn engine(party: u8) -> Engine {
        let setup = RunSetup {
            build_mode: BuildMode::Equipment,
            starting_rune: Some("thunder".into()),
            ..RunSetup::default()
        };
        Engine::start_party(&setup, party).unwrap()
    }

    fn refused(what: &'static str) -> Result<(), DecodeError> {
        Err(DecodeError::Invalid(what))
    }

    /// T25: for every present slot the layout, the kinds and the runes are checked.
    #[test]
    fn the_savepoint_refuses_a_bad_slot_layout() {
        let e = engine(2);
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let counted = e.saved_parts().1;
        let check = |s: &State| check_shape(s, counted, state, counted);
        assert_eq!(check(state), Ok(()), "the fresh state is accepted");
        for me in state.players.indices() {
            let edit = |f: &dyn Fn(&mut State)| {
                let mut s = state.clone();
                f(&mut s);
                check(&s)
            };
            let slots =
                |s: &mut State| std::mem::take(&mut s.players.career_mut(me).equipment.slots);
            let layout = "equipment slots differ from the layout";
            let kind = "equipment slot holds the wrong kind";
            // Fewer or more slots, or none.
            assert_eq!(
                edit(&|s| {
                    s.players.career_mut(me).equipment.slots.pop();
                }),
                refused(layout),
                "{me:?}"
            );
            assert_eq!(
                edit(&|s| {
                    let extra = s.players.career(me).equipment.slots[0].clone();
                    s.players.career_mut(me).equipment.slots.push(extra);
                }),
                refused(layout)
            );
            assert_eq!(edit(&|s| drop(slots(s))), refused(layout));
            // A slot of another kind.
            assert_eq!(
                edit(&|s| s.players.career_mut(me).equipment.slots[0].kind =
                    crate::sim::equipment::SlotKind::Rune),
                refused(layout)
            );
            // A rune in a category slot, a star in a rune slot.
            assert_eq!(
                edit(&|s| s.players.career_mut(me).equipment.slots[0].held =
                    Some(Held::Rune(Rune::Bleed))),
                refused(kind)
            );
            assert_eq!(
                edit(&|s| s.players.career_mut(me).equipment.slots[10].held = Some(Held::Star(0))),
                refused(kind)
            );
            // The runes the Main fires are the rune slots filled, in order.
            let runes = "runes differ from the rune slots";
            assert_eq!(
                edit(&|s| s.players.career_mut(me).equipment.slots[10].held =
                    Some(Held::Rune(Rune::Bleed))),
                refused(runes)
            );
            assert_eq!(edit(&|s| s.players[me].runes.clear()), refused(runes));
            // A consistent equip is accepted: the rune slot and the list agree.
            assert_eq!(
                edit(&|s| {
                    s.players.career_mut(me).equipment.slots[10].held =
                        Some(Held::Rune(Rune::Bleed));
                    s.players[me].runes.push(HeldRune {
                        rune: Rune::Bleed,
                        cooldown_left: 7,
                    });
                }),
                Ok(())
            );
        }
    }

    // --- a star must fit its slot: its category, and no two share an option (ADR-SAVE-4) --------

    const DOES_NOT_FIT: &str = "equipment star does not fit its slot's category";
    const SHARED: &str = "equipment stars share an option";

    // The bundled layout: Attack 0..5, Defence 5..7, Mobility 7..9, runes 9..12, Lumine 12.
    const LUMINE: usize = 12;

    /// The place of the reward `id` in the bundled file.
    fn place(e: &Engine, id: &str) -> u16 {
        let at = e.content().rewards().index_of(&id.into()).expect(id);
        u16::try_from(at).unwrap()
    }

    /// `state` with the star `id` put into slot `at` for each pair, for every player.
    fn with(state: &State, puts: &[(usize, &str)], e: &Engine) -> State {
        let mut s = state.clone();
        for me in s.players.indices() {
            let eq = &mut s.players.career_mut(me).equipment;
            for &(at, id) in puts {
                let slot = if at == LUMINE {
                    eq.lumine.as_mut().expect("the Lumine slot")
                } else {
                    &mut eq.slots[at]
                };
                slot.held = Some(Held::Star(place(e, id)));
                slot.level = 1;
            }
        }
        s
    }

    fn lumine_engine() -> Engine {
        let setup = RunSetup {
            build_mode: BuildMode::Equipment,
            lumine: 4,
            lumine_star: Some("attack_common".into()),
            ..RunSetup::default()
        };
        Engine::start_party(&setup, 2).unwrap()
    }

    fn indices(e: &Engine, s: &State) -> Result<(), DecodeError> {
        let Run::V2 { world, .. } = e.run() else {
            panic!("the live rules")
        };
        check_indices(s, world, e.content())
    }

    /// C1: a star of another category in a category slot is refused, for both players.
    #[test]
    fn a_star_of_another_category_is_refused() {
        let e = engine(2);
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        assert_eq!(indices(&e, state), Ok(()));
        for (at, id) in [
            (0, "max_hp_common"),
            (5, "attack_common"),
            (7, "max_hp_rare"),
        ] {
            assert_eq!(
                indices(&e, &with(state, &[(at, id)], &e)),
                refused(DOES_NOT_FIT),
                "slot {at} {id}"
            );
        }
    }

    /// C2: a star with no category (a Growth star) is refused in a category slot.
    #[test]
    fn a_category_less_star_in_a_category_slot_is_refused() {
        let e = engine(2);
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let growth = usize::from(place(&e, "xp_common"));
        assert!(e.content().rewards().rewards()[growth].category.is_none());
        for at in [0, 5, 7] {
            assert_eq!(
                indices(&e, &with(state, &[(at, "xp_common")], &e)),
                refused(DOES_NOT_FIT),
                "slot {at}"
            );
        }
    }

    /// C3: two stars of one option are refused, in two category slots or with the Lumine slot.
    #[test]
    fn two_stars_sharing_an_option_are_refused() {
        let e = lumine_engine();
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let two = with(state, &[(5, "max_hp_common"), (6, "max_hp_rare")], &e);
        assert_eq!(indices(&e, &two), refused(SHARED));
        let with_lumine = with(state, &[(5, "max_hp_common"), (LUMINE, "max_hp_rare")], &e);
        assert_eq!(indices(&e, &with_lumine), refused(SHARED));
        // The setup's Lumine star (attack) against another grade of it in an attack slot.
        let attack = with(state, &[(0, "attack_rare")], &e);
        assert_eq!(indices(&e, &attack), refused(SHARED));
    }

    /// C4: one star per option, each in its category, the Lumine slot any category: accepted.
    #[test]
    fn a_legal_set_is_accepted() {
        let e = lumine_engine();
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let puts = [
            (0, "attack_speed_common"),
            (5, "max_hp_common"),
            (7, "speed_common"),
            (LUMINE, "xp_common"),
        ];
        let ok = with(state, &puts, &e);
        assert_eq!(indices(&e, &ok), Ok(()));
        // An equipped star equal to an inventory star is not a clash.
        let mut s = ok.clone();
        for me in s.players.indices() {
            let held = s.players.career(me).equipment.slots[5].held.unwrap();
            s.players.career_mut(me).equipment.inventory.push(held);
        }
        assert_eq!(indices(&e, &s), Ok(()));
    }

    /// C5: the load path (`decode_state`) refuses the misplaced star; the unmodified state loads.
    #[test]
    fn the_load_refuses_through_decode_state() {
        use super::super::{Encode, Writer};
        let e = engine(2);
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        let (ledger, counted, stats) = e.saved_parts();
        let bytes_of = |s: &State| {
            let mut w = Writer::new();
            stats.encode(&mut w);
            let mut bytes = w.finish();
            bytes.extend(super::v2::encode(s, ledger, counted));
            bytes
        };
        let fresh = engine(2);
        assert!(decode_state(&bytes_of(state), &fresh).is_ok(), "unmodified");
        let bad = with(state, &[(0, "max_hp_common")], &e);
        assert_eq!(
            decode_state(&bytes_of(&bad), &fresh).err(),
            Some(DecodeError::Invalid(DOES_NOT_FIT))
        );
        let clash = with(state, &[(5, "max_hp_common"), (6, "max_hp_rare")], &e);
        assert_eq!(
            decode_state(&bytes_of(&clash), &fresh).err(),
            Some(DecodeError::Invalid(SHARED))
        );
    }
}

#[cfg(test)]
mod offers_tests {
    //! Slot 0's star offers and offer generator sit where the star field and the run's generator
    //! were: they round-trip through format 1 and the shape check names the offers' lists.

    use crate::bench_fixtures::{offers_cases, time_limit_case, worst_case};
    use crate::engine::{Engine, Run};
    use crate::id::PlayerIdx;
    use crate::savepoint::DecodeError;
    use crate::savepoint::state::{decode_state, encode_state};

    use super::v2::check_shape;

    /// `e` stepped until a star offer is open, with one reroll spent when the offer has one.
    fn with_open_offer(mut e: Engine, input: fn(u32) -> crate::input::Input) -> Option<Engine> {
        for _ in 0..3_000 {
            let tick = e.tick() + 1;
            e.step(&[input(tick), Default::default(), Default::default()]);
            let Run::V2 { state, .. } = e.run() else {
                panic!("the live rules")
            };
            let offers = state.players.career(PlayerIdx::SOLO).offers.as_ref()?;
            if let Some(o) = &offers.offer {
                let n = u32::try_from(o.candidates.len()).unwrap();
                if offers.rerolls_left > 0 {
                    let _ = crate::engine::pick(&mut e, PlayerIdx::SOLO, n);
                }
                return Some(e);
            }
        }
        None
    }

    #[test]
    fn slot_0s_offers_and_generator_round_trip() {
        let mut opened = 0;
        for (e, stage, input) in [
            (
                offers_cases::worst_warmed_up(),
                offers_cases::worst_stage(),
                worst_case::input as fn(u32) -> _,
            ),
            (
                offers_cases::time_limit_tick().engine,
                time_limit_case::stage(),
                time_limit_case::input,
            ),
        ] {
            let fresh = Engine::new_with(&stage, e.content()).unwrap();
            let Some(e) = with_open_offer(e, input) else {
                continue;
            };
            opened += 1;
            let bytes = encode_state(&e);
            let (saved, ledger, counted, stats) = decode_state(&bytes, &fresh).unwrap();
            let back = fresh.clone().with_saved(saved, ledger, counted, stats);
            let (Run::V2 { state: a, .. }, Run::V2 { state: b, .. }) = (e.run(), back.run()) else {
                panic!("the live rules")
            };
            let (ca, cb) = (
                a.players.career(PlayerIdx::SOLO),
                b.players.career(PlayerIdx::SOLO),
            );
            assert!(ca.offers.as_ref().unwrap().offer.is_some());
            assert_eq!(ca.offers, cb.offers);
            assert_eq!(ca.offer, cb.offer);
            assert_eq!(
                encode_state(&back),
                bytes,
                "re-encoding gives the same bytes"
            );
        }
        assert!(opened > 0, "no case opened a star offer");
    }

    #[test]
    fn the_shape_check_refuses_an_offers_list_of_the_wrong_length() {
        let e = offers_cases::worst_warmed_up();
        let fresh = Engine::new_with(&offers_cases::worst_stage(), e.content()).unwrap();
        let (Run::V2 { state: s, .. }, Run::V2 { state: blank, .. }) = (e.run(), fresh.run())
        else {
            panic!("the live rules")
        };
        let counted = e.saved_parts().1;
        let mut bad = s.clone();
        bad.players
            .career_mut(PlayerIdx::SOLO)
            .offers
            .as_mut()
            .expect("stars")
            .taken
            .push(1);
        assert_eq!(
            check_shape(&bad, counted, blank, counted),
            Err(DecodeError::Invalid(
                "star taken length differs from the content"
            ))
        );
    }
}

#[cfg(test)]
mod career_rest_tests {
    //! Slot 0's expertise gauge, Stardust and regeneration sit where the run's single ones were:
    //! they round-trip through format 1 and the shape check names their presence and lengths.

    use crate::bench_fixtures::{offers_cases, time_limit_case};
    use crate::engine::{Engine, Run};
    use crate::id::PlayerIdx;
    use crate::savepoint::DecodeError;
    use crate::savepoint::state::{decode_state, encode_state};

    use super::v2::check_shape;

    #[test]
    fn slot_0s_gauge_stardust_and_regen_round_trip() {
        let mut gauges = 0;
        for (e, stage) in [
            (offers_cases::worst_warmed_up(), offers_cases::worst_stage()),
            (
                offers_cases::time_limit_tick().engine,
                time_limit_case::stage(),
            ),
        ] {
            let fresh = Engine::new_with(&stage, e.content()).unwrap();
            // The cases earn nothing in their warm-up, so the values are set on a copy.
            let (mut saved, ledger, counted, stats) =
                decode_state(&encode_state(&e), &fresh).unwrap();
            let crate::engine::Saved::V2(state) = &mut saved else {
                panic!("the live rules")
            };
            let c = state.players.career_mut(PlayerIdx::SOLO);
            if let Some(g) = c.expertise.as_mut() {
                gauges += 1;
                g.points = 4;
                g.level = 2;
                g.range_pct = 6;
            }
            if c.stardust.is_some() {
                c.stardust = Some(1_230);
            }
            c.regen = 3;
            let (want_gauge, want_dust) = (c.expertise.clone(), c.stardust);
            let set = fresh.clone().with_saved(saved, ledger, counted, stats);
            let bytes = encode_state(&set);
            let (saved, ledger, counted, stats) = decode_state(&bytes, &fresh).unwrap();
            let back = fresh.clone().with_saved(saved, ledger, counted, stats);
            let Run::V2 { state: b, .. } = back.run() else {
                panic!("the live rules")
            };
            let d = b.players.career(PlayerIdx::SOLO);
            assert_eq!(d.expertise, want_gauge);
            assert_eq!(d.stardust, want_dust);
            assert_eq!(d.regen, 3);
            assert_eq!(encode_state(&back), bytes);
        }
        assert!(gauges > 0, "no case has a gauge");
    }

    #[test]
    fn the_shape_check_names_the_gauge_and_stardust() {
        let mut checked = 0;
        for (e, stage) in [
            (offers_cases::worst_warmed_up(), offers_cases::worst_stage()),
            (
                offers_cases::time_limit_tick().engine,
                time_limit_case::stage(),
            ),
        ] {
            let fresh = Engine::new_with(&stage, e.content()).unwrap();
            let (Run::V2 { state, .. }, Run::V2 { state: blank, .. }) = (e.run(), fresh.run())
            else {
                panic!("the live rules")
            };
            let counted = e.saved_parts().1;
            let mut bad = state.clone();
            let c = bad.players.career_mut(PlayerIdx::SOLO);
            if let Some(g) = c.expertise.as_mut() {
                checked += 1;
                g.taken.push(0);
                assert_eq!(
                    check_shape(&bad, counted, blank, counted),
                    Err(DecodeError::Invalid(
                        "expertise taken length differs from the content"
                    ))
                );
                bad.players.career_mut(PlayerIdx::SOLO).expertise = None;
                assert_eq!(
                    check_shape(&bad, counted, blank, counted),
                    Err(DecodeError::Invalid(
                        "expertise presence differs from the content"
                    ))
                );
            }
            let mut bad = state.clone();
            let c = bad.players.career_mut(PlayerIdx::SOLO);
            c.stardust = match c.stardust {
                Some(_) => None,
                None => Some(0),
            };
            assert_eq!(
                check_shape(&bad, counted, blank, counted),
                Err(DecodeError::Invalid(
                    "stardust presence differs from the content"
                ))
            );
        }
        assert!(checked > 0, "no case has a gauge");
    }
}

#[cfg(test)]
mod frozen_text_tests {
    //! Version 1's coding is the macro's text and its invocation; one moved byte there fails this
    //! (ADR-REPL-2). The markers are built in pieces so this test does not contain them.

    use crate::hash::fnv1a64;

    /// FNV-1a 64 of the text from the macro's first line through version 1's invocation.
    const V1_TEXT_HASH: u64 = 13_219_549_500_252_208_330;

    #[test]
    fn version_1_text_is_unchanged() {
        let src = include_str!("state.rs");
        let start_marker = concat!("macro_rules! state_codec", " {\n");
        let end_marker = concat!(
            "mod v1",
            " {\n    state_codec!(sim_v1, |s| vec![&s.player]);\n}\n"
        );
        let start = src.find(start_marker).expect("the macro");
        let end = src.find(end_marker).expect("version 1") + end_marker.len();
        assert_eq!(fnv1a64(&src.as_bytes()[start..end]), V1_TEXT_HASH);
    }
}
