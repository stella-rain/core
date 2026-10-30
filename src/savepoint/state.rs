//! `encode_state` and `decode_state`: the whole run state as savepoint bytes, and back.
//!
//! The bytes are the `State`, the `RunLedger` and the `Counted`, in that order, with no header
//! (the file's is the savepoint file's) and no trailing bytes. What the content and the setup
//! decide is not in them: `decode_state` takes it from a fresh engine of the same run, and
//! checks every length and index the bytes hold against it, so a stale or hostile file is
//! refused, not indexed out of range on a later tick (ADR-SVC-1).

use crate::engine::Engine;
use crate::ledger::{Counted, RunLedger};
use crate::rewards::ORBIT_SET_BASE;
use crate::sim::state::State;

use super::{Decode, DecodeError, Encode, Reader, Writer};

/// The state, the ledger and the tallies of `engine` as bytes.
#[allow(dead_code)] // the savepoint file calls it next
pub(crate) fn encode_state(engine: &Engine) -> Vec<u8> {
    let (_, _, state, ledger, counted) = engine.savepoint_parts();
    let mut w = Writer::new();
    state.encode(&mut w);
    ledger.encode(&mut w);
    counted.encode(&mut w);
    w.finish()
}

/// The state, the ledger and the tallies in `bytes`, for a run of `fresh`'s content. `fresh` is
/// an engine started from the same setup and content, untouched: everything the content and the
/// setup decide comes from it, and every length and index in the bytes is checked against it.
#[allow(dead_code)] // the savepoint file calls it next
pub(crate) fn decode_state(
    bytes: &[u8],
    fresh: &Engine,
) -> Result<(State, RunLedger, Counted), DecodeError> {
    let (world, content, fresh_state, fresh_ledger, fresh_counted) = fresh.savepoint_parts();
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
    check_indices(&state, fresh)?;

    // Only now are the runs known to index real attacks and phases.
    state.move_index.rebuild(world, &state.runs);
    crate::sim::reserve(world, &mut state);
    let _ = content;
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
fn check_indices(s: &State, fresh: &Engine) -> Result<(), DecodeError> {
    let (world, content, _, _, _) = fresh.savepoint_parts();
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
    for o in &s.player.orbits {
        // The key of an orbit is its reward's place, or `ORBIT_SET_BASE` and a set's place.
        let in_range = match o.reward.checked_sub(ORBIT_SET_BASE) {
            None => usize::from(o.reward) < rewards,
            Some(set) => usize::from(set) < sets,
        };
        if !in_range {
            return Err(DecodeError::Invalid("orbit reward out of range"));
        }
    }
    if let Some(slot) = &s.player.skill {
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
