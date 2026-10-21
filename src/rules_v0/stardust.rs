//! Stardust (ADR-109): every kill by the player earns its base Stardust (times 10, from the
//! wave, elite type or boss that made the enemy) times the time factor at the kill tick, in
//! integer percent truncated toward zero, added to the run's counter, which saturates at
//! `MAX_RUN_STARDUST`. The factor is `start_pct` at tick 0 and `end_pct` at the run's length (a
//! mode run's length without its overtime), in a straight line between and held at `end_pct`
//! after. Nothing random is drawn. Summoned enemies, guardians, escorts and a boss's parts earn
//! nothing: only a wave's enemies (elite types are waves here) and the boss do. A stage without
//! a factor counts nothing.

use crate::engine::{AgentSource, State};
use crate::stage::{Stage, StardustFactor};
use crate::validate::limits::MAX_RUN_STARDUST;

/// The factor at `tick` of a run `length` ticks long, in percent, truncated.
pub(crate) fn factor_pct(f: &StardustFactor, tick: u32, length: u32) -> u32 {
    let length = u64::from(length.max(1));
    let at = u64::from(tick).min(length);
    let rise = u64::from(f.end_pct.saturating_sub(f.start_pct));
    // `at <= length`, so the step is at most `rise`, which fits a `u32`.
    u32::try_from(u64::from(f.start_pct) + rise * at / length).unwrap_or(u32::MAX)
}

/// Adds a kill of base `base` at this tick to the counter.
fn earn(stage: &Stage, s: &mut State, base: u32) {
    let (Some(f), Some(total)) = (stage.stardust.as_ref(), s.stardust) else {
        return;
    };
    let length = s
        .horde
        .as_ref()
        .map_or(stage.length_ticks, |h| h.length_ticks);
    let pct = factor_pct(f, s.tick, length);
    let gained = u64::from(base) * u64::from(pct) / 100;
    let sum = u64::from(total) + gained;
    s.stardust = Some(u32::try_from(sum.min(u64::from(MAX_RUN_STARDUST))).unwrap_or(u32::MAX));
}

/// The enemies at 0 hp, before they are removed.
pub(super) fn credit_defeated(stage: &Stage, s: &mut State) {
    if s.stardust.is_none() {
        return;
    }
    let bases: Vec<u32> = s
        .enemies
        .iter()
        .filter(|e| e.hp == 0 && e.key.source == AgentSource::Wave)
        .filter_map(|e| {
            usize::try_from(e.key.index)
                .ok()
                .and_then(|i| stage.waves.get(i))
                .and_then(|w| w.stardust)
        })
        .collect();
    for base in bases {
        earn(stage, s, base);
    }
}

/// The boss has just died.
pub(super) fn boss_killed(stage: &Stage, s: &mut State) {
    if let Some(base) = stage.boss.as_ref().and_then(|b| b.stardust) {
        earn(stage, s, base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: StardustFactor = StardustFactor {
        start_pct: 200,
        end_pct: 400,
    };

    #[test]
    fn the_factor_runs_from_start_to_end_in_a_line_truncated() {
        assert_eq!(factor_pct(&F, 0, 1_000), 200);
        assert_eq!(factor_pct(&F, 250, 1_000), 250);
        assert_eq!(factor_pct(&F, 333, 1_000), 266);
        assert_eq!(factor_pct(&F, 1_000, 1_000), 400);
    }

    #[test]
    fn past_the_length_it_holds_at_the_end() {
        assert_eq!(factor_pct(&F, 5_000, 1_000), 400);
        assert_eq!(factor_pct(&F, u32::MAX, 1), 400);
    }

    #[test]
    fn the_largest_numbers_do_not_overflow() {
        let f = StardustFactor {
            start_pct: 1,
            end_pct: u32::MAX,
        };
        assert_eq!(factor_pct(&f, u32::MAX, u32::MAX), u32::MAX);
    }
}
