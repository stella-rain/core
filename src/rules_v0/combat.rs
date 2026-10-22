//! The named stars' combat rewards (core#NS2b, app#336): critical hits, echoes of hits and kill
//! bursts. They act on the main character's own hits only: a hit of a rune, of a
//! damage-over-time status, an echo and a burst neither crit nor echo.
//!
//! A run without any of the rewards runs none of this: every entry point returns at once, the
//! state writes nothing for them to the hash and no random number is drawn.

use crate::engine::PlayerState;
use crate::id::EntityId;
use crate::rewards::{MAX_CRIT_CHANCE_TOTAL_PCT, MAX_CRIT_DAMAGE_TOTAL_PCT};
use crate::rng::SplitMix64;

use super::{PLAYER_ID, Shot, State};

/// Whether any combat reward is held: what the state hash writes them for.
pub(super) fn held(p: &PlayerState) -> bool {
    p.crit_chance_pct > 0 || p.crit_damage_pct > 0 || p.echo_hit_pct > 0 || p.burst_pct > 0
}

/// Whether `b`, a hit on the foe `id` that has `hp` hit points before it, is a critical hit.
/// Only the main character's own hits on enemies, the boss and its parts roll, and only with
/// crit chance held. The roll is a hash of the run's seed, the tick, the target, its hit points
/// and where the shot landed, so two hits on one target in one tick differ and no draw is
/// taken from the run's generator.
pub(super) fn crits(s: &State, b: &Shot, id: EntityId, hp: u32, foe: bool) -> bool {
    let chance = s.player.crit_chance_pct.min(MAX_CRIT_CHANCE_TOTAL_PCT);
    if chance == 0 || b.chain || b.rune.is_some() || !b.friendly || b.owner != PLAYER_ID || !foe {
        return false;
    }
    let parts = [
        u64::from(s.tick),
        u64::from(id.0),
        u64::from(hp),
        u64::from(b.at.x.0 as u32),
        u64::from(b.at.y.0 as u32),
    ];
    let mut h = s.crit_seed;
    for part in parts {
        h = SplitMix64::new(h ^ part).next_u64();
    }
    h % 100 < u64::from(chance)
}

/// `damage` of a critical hit: twice as much, and `bonus_pct` percent more (held to
/// `MAX_CRIT_DAMAGE_TOTAL_PCT`).
pub(super) fn crit_damage(damage: u32, bonus_pct: u32) -> u32 {
    let pct = 200 + u64::from(bonus_pct.min(MAX_CRIT_DAMAGE_TOTAL_PCT));
    u32::try_from(u64::from(damage) * pct / 100).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::Point;
    use crate::rewards::Rarity;
    use crate::rules_v0::{RunDifficulty, RunPool, initial_state};
    use crate::run_content::RunContent;

    fn state() -> State {
        let stage = RunContent::bundled().trial().clone();
        let pool = RunPool {
            rewards: Vec::new(),
            extra_rerolls: 0,
            upgrades: 0,
            difficulty: RunDifficulty {
                hp_pct: 100,
                damage_pct: 100,
                rarity: (Rarity::Common, Rarity::Supernova),
            },
        };
        initial_state(&stage, RunContent::bundled(), pool, 1)
    }

    fn shot(owner: EntityId) -> Shot {
        Shot {
            at: Point::default(),
            owner,
            friendly: true,
            damage: 100,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        }
    }

    #[test]
    fn a_crit_damage_is_twice_the_damage_and_the_bonus() {
        assert_eq!(crit_damage(100, 0), 200);
        assert_eq!(crit_damage(100, 100), 300);
        assert_eq!(crit_damage(100, 300), 500);
        // Held to the total even if an unvalidated sum passes it.
        assert_eq!(crit_damage(100, u32::MAX), 500);
        assert_eq!(crit_damage(u32::MAX, 300), u32::MAX);
    }

    fn rolls(s: &mut State, b: Shot) -> usize {
        (0..1_000u32)
            .filter(|t| {
                s.tick = *t;
                crits(s, &b, EntityId(1), 500, true)
            })
            .count()
    }

    #[test]
    fn a_roll_draws_nothing_from_the_runs_generator() {
        let mut s = state();
        s.player.crit_chance_pct = 50;
        let before = s.rng.state();
        assert!(rolls(&mut s, shot(PLAYER_ID)) > 0);
        assert_eq!(s.rng.state(), before);
    }

    #[test]
    fn the_roll_follows_the_chance_and_only_the_main_characters_own_hits_on_foes() {
        let mut s = state();
        assert_eq!(rolls(&mut s, shot(PLAYER_ID)), 0, "no chance held");
        s.player.crit_chance_pct = 50;
        let half = rolls(&mut s, shot(PLAYER_ID));
        assert!((400..=600).contains(&half), "{half} of 1000");
        s.player.crit_chance_pct = u32::MAX;
        let most = rolls(&mut s, shot(PLAYER_ID));
        assert!((650..=850).contains(&most), "held to 75: {most}");
        let mut other = shot(PLAYER_ID);
        other.chain = true;
        assert_eq!(rolls(&mut s, other), 0, "a chained hit never crits");
        let mut rune = shot(PLAYER_ID);
        rune.rune = Some(crate::rune::Rune::ALL[0]);
        assert_eq!(rolls(&mut s, rune), 0, "nor a rune's");
        assert_eq!(rolls(&mut s, shot(EntityId(9))), 0, "nor another's hit");
        assert!(
            !crits(&s, &shot(PLAYER_ID), EntityId(1), 500, false),
            "nor on an ally"
        );
    }
}
