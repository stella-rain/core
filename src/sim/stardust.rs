//! Stardust: every kill earns, for each player that is up (ADR-SIM-7 point 5; a downed or out
//! player earns none and keeps what it has), its base Stardust (times 10, from the
//! wave, elite type or boss that made the enemy) times the time factor at the kill tick, in
//! integer percent truncated toward zero, added to the run's counter, which saturates at
//! `MAX_RUN_STARDUST`. The factor is the percent of the last tier of the stage's table that
//! started at or before the kill tick; past the last tier's start, overtime
//! included, the last tier holds. Nothing random is drawn. A wave's enemies (elite types are waves
//! here: the Treasure Goblin's kill, a shrine's win, which sets its hit points to 0) and the boss
//! earn their own base; an enemy summoned by a boss or a splitter earns the factor's
//! `summon_base`. Guardians, escorts and a boss's parts earn nothing. A stage without a factor
//! counts nothing.

use super::World;
use crate::run_stats::StatEffect;
use crate::sim::state::{AgentSource, PlayerIdx, State};
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
fn earn(stage: &World, s: &mut State, me: PlayerIdx, base: u32) {
    let (Some(f), Some(total)) = (stage.stardust.as_ref(), s.players.career(me).stardust) else {
        return;
    };
    let pct = factor_pct(f, s.tick);
    // The stat's percent adds to 100 and the product is cut once; at a total of 0 this is
    // `base * pct / 100`.
    let more = 100 + u128::from(super::levels::total(s, me, StatEffect::StardustPct));
    let gained =
        u64::try_from(u128::from(base) * u128::from(pct) * more / 10_000).unwrap_or(u64::MAX);
    let sum = u64::from(total) + gained;
    s.players.career_mut(me).stardust =
        Some(u32::try_from(sum.min(u64::from(MAX_RUN_STARDUST))).unwrap_or(u32::MAX));
}

/// The enemies at 0 hp, before they are removed.
pub(super) fn credit_defeated(stage: &World, s: &mut State, me: PlayerIdx) {
    if !s.players[me].is_up() {
        return;
    }
    if s.players.career(me).stardust.is_none() {
        return;
    }
    let summon_base = stage.stardust.as_ref().and_then(|f| f.summon_base);
    // The bases wait in the tick's scratch: `earn` needs `s` whole.
    let mut bases = std::mem::take(&mut s.scratch.bases);
    bases.clear();
    bases.extend(
        s.enemies
            .iter()
            .filter(|e| e.hp == 0 && !e.burst)
            .filter_map(|e| match e.key.source {
                AgentSource::Wave => usize::try_from(e.key.index)
                    .ok()
                    .and_then(|i| stage.waves.get(i))
                    .and_then(|w| w.stardust),
                AgentSource::Summoned | AgentSource::BossRule => summon_base,
                _ => None,
            }),
    );
    for &base in &bases {
        earn(stage, s, me, base);
    }
    s.scratch.bases = bases;
}

/// The boss has just died.
pub(super) fn boss_killed(stage: &World, s: &mut State, me: PlayerIdx) {
    if !s.players[me].is_up() {
        return;
    }
    if let Some(base) = s.boss_def(stage).and_then(|b| b.stardust) {
        earn(stage, s, me, base);
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

    use crate::sim::party_fix_tests::{party, slot};

    /// A solo run counting Stardust at factor `pct`, with the `StardustPct` total `total`.
    fn earning(pct: u32, total: Option<u32>) -> (World, State) {
        let (mut w, mut s) = party(1);
        w.stage.stardust = Some(table(&[(0, pct)]));
        let career = s.players.career_mut(slot(0));
        career.stardust = Some(0);
        match total {
            Some(t) => {
                career.progress.as_mut().expect("levels").totals[StatEffect::StardustPct as usize] =
                    t
            }
            None => career.progress = None,
        }
        (w, s)
    }

    fn earned(pct: u32, total: Option<u32>, base: u32) -> u32 {
        let (w, mut s) = earning(pct, total);
        earn(&w, &mut s, slot(0), base);
        s.players.career(slot(0)).stardust.expect("counted")
    }

    /// T14: the stat scales a kill once, with a single truncation.
    #[test]
    fn stardust_pct_scales_a_kill_once() {
        assert_eq!(earned(100, Some(0), 40), 40);
        assert_eq!(earned(100, Some(50), 40), 60);
        assert_eq!(earned(150, Some(0), 5), 7);
        assert_eq!(earned(150, Some(100), 5), 15);
    }

    /// T15: no overflow, the cap holds, and no `Progress` means no stat.
    #[test]
    fn stardust_pct_cannot_overflow_or_pass_the_cap() {
        assert_eq!(
            earned(1000, Some(u32::MAX), u32::MAX),
            MAX_RUN_STARDUST,
            "the counter holds at the cap"
        );
        assert_eq!(earned(100, None, 40), 40);
    }
}
