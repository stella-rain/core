//! Stardust (ADR-109): every kill by the player earns its base Stardust (times 10, from the
//! wave, elite type or boss that made the enemy) times the time factor at the kill tick, in
//! integer percent truncated toward zero, added to the run's counter, which saturates at
//! `MAX_RUN_STARDUST`. The factor is the percent of the last tier of the stage's table that
//! started at or before the kill tick (ADR-109 decision 2); past the last tier's start, overtime
//! included, the last tier holds. Nothing random is drawn. A wave's enemies (elite types are waves
//! here: the Treasure Goblin's kill, a shrine's win, which sets its hit points to 0) and the boss
//! earn their own base; an enemy summoned by a boss or a splitter earns the factor's
//! `summon_base`. Guardians, escorts and a boss's parts earn nothing. A stage without a factor
//! counts nothing.

use super::Run;
use crate::engine::{AgentSource, State};
use crate::stage::StardustFactor;
use crate::validate::limits::MAX_RUN_STARDUST;

/// The factor at `tick`, in percent: the last tier that started at or before it. The validator
/// makes the first tier start at 0; a table without one counts 0 before its first tier.
pub(crate) fn factor_pct(f: &StardustFactor, tick: u32) -> u32 {
    f.tiers
        .iter()
        .take_while(|t| t.start_tick <= tick)
        .last()
        .map_or(0, |t| t.pct)
}

/// Adds a kill of base `base` at this tick to the counter.
fn earn(stage: &Run, s: &mut State, base: u32) {
    let (Some(f), Some(total)) = (stage.stardust.as_ref(), s.stardust) else {
        return;
    };
    let pct = factor_pct(f, s.tick);
    let gained = u64::from(base) * u64::from(pct) / 100;
    let sum = u64::from(total) + gained;
    s.stardust = Some(u32::try_from(sum.min(u64::from(MAX_RUN_STARDUST))).unwrap_or(u32::MAX));
}

/// The enemies at 0 hp, before they are removed.
pub(super) fn credit_defeated(stage: &Run, s: &mut State) {
    if s.stardust.is_none() {
        return;
    }
    let summon_base = stage.stardust.as_ref().and_then(|f| f.summon_base);
    let bases: Vec<u32> = s
        .enemies
        .iter()
        .filter(|e| e.hp == 0 && !e.burst)
        .filter_map(|e| match e.key.source {
            AgentSource::Wave => usize::try_from(e.key.index)
                .ok()
                .and_then(|i| stage.waves.get(i))
                .and_then(|w| w.stardust),
            AgentSource::Summoned | AgentSource::BossRule => summon_base,
            _ => None,
        })
        .collect();
    for base in bases {
        earn(stage, s, base);
    }
}

/// The boss has just died.
pub(super) fn boss_killed(stage: &Run, s: &mut State) {
    if let Some(base) = s.boss_def().and_then(|b| b.stardust) {
        earn(stage, s, base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::StardustTier;

    fn table(list: &[(u32, u32)]) -> StardustFactor {
        StardustFactor {
            tiers: list
                .iter()
                .map(|&(start_tick, pct)| StardustTier { start_tick, pct })
                .collect(),
            summon_base: None,
        }
    }

    #[test]
    fn the_factor_is_the_last_tier_started_by_the_tick() {
        let f = table(&[(0, 100), (18_000, 110), (36_000, 120)]);
        assert_eq!(factor_pct(&f, 0), 100);
        assert_eq!(factor_pct(&f, 17_999), 100);
        assert_eq!(factor_pct(&f, 18_000), 110);
        assert_eq!(factor_pct(&f, 35_999), 110);
        assert_eq!(factor_pct(&f, 36_000), 120);
    }

    #[test]
    fn past_the_last_tier_it_holds() {
        let f = table(&[(0, 200), (10, 400)]);
        assert_eq!(factor_pct(&f, 5_000), 400);
        assert_eq!(factor_pct(&f, u32::MAX), 400);
    }
}
