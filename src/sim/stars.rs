//! Falling stars and their offers (ADR-REPL-5), in a stage with `stars`.
//!
//! In the collisions, after the defeated enemies' experience, each defeated enemy in entity-ID
//! order may drop a star where it fell. With the map full of stars it drops none, and the pity
//! does not count it. Otherwise it drops one for sure once `pity` kills in a row have dropped
//! none, and on a draw of the drop chance before that. A dropped star's rarity is drawn from the
//! weights of the drop table's tier for the player's level, after this tick's experience. An
//! elite drops one for sure, without a draw and leaving the pity as it was, from the
//! next tier up, or from the last tier. Then the main character takes every star inside its
//! pickup circle, in the order they dropped.
//!
//! A boss kill gives its equipment bonus (`starlight::boss_bonus`) and the boss's drops
//! (`drops::boss_killed`); it makes no offer due.
//!
//! The live rules open no star offer: a star taken, and a boss kill, add nothing due. A boss kill
//! still gives its equipment bonus, and what a star is worth comes from the equipped stars
//! (`build.rs`). Star offers exist only in the frozen rules of `sim_version` 1 (`sim_v1`).
//!
//! A reward's effects are the build alone: what the stars held add up to (`build.rs`, a function
//! of the counts held).
//!
//! The star field is two parts: the run's (`StarField`: the stars on the map, the pity, the
//! rarity range, the rune and reward pools) and each player's (`Offers` in the `Career`: what
//! is left of the player's offers, the star-set counts). The
//! hash and the savepoint write the run's part with the shared fields and each present player's
//! in its block.
//! The drops are per slot: a star taken gives the player that took it a roll from its own
//! generator, and a boss kill gives every player that is up one, at one tier. The drops on the
//! map are shared, and a drop's tier is the highest level among the present players.

use super::World;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{StarState, State};
use crate::snapshot::ConstellationView;
use crate::stage::{RarityWeights, Stars, Tier};
use crate::validate::limits::MAX_STARS_ON_MAP;

/// The drop chance is out of this.
const PER_MILLE: u32 = 1000;

/// Stars for the enemies at 0 hp, before they are removed.
pub(super) fn drop_from_defeated(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.stars else {
        return;
    };
    let Some(tier) = tier_of(cfg, top_level(s)) else {
        return;
    };
    let cap = usize::from(cfg.cap.min(MAX_STARS_ON_MAP));
    // An elite's star draws from the next tier up, or from the last.
    let up = cfg
        .tiers
        .iter()
        .position(|t| std::ptr::eq(t, tier))
        .and_then(|i| cfg.tiers.get(i + 1))
        .unwrap_or(tier);
    let State {
        rng,
        stars,
        next_id,
        enemies,
        scratch,
        ..
    } = s;
    let Some(f) = stars.as_mut() else {
        return;
    };
    scratch.fallen.clear();
    scratch.fallen.extend(
        enemies
            .iter()
            .filter(|e| e.hp == 0 && !e.burst)
            .map(|e| (e.at, e.elite.is_some())),
    );
    for &(at, elite) in &scratch.fallen {
        if f.on_map.len() >= cap {
            continue;
        }
        // An elite drops its star for sure, and leaves the pity as it was.
        if !elite {
            let drops = f.misses >= u32::from(cfg.pity)
                || rng.loot.below(PER_MILLE) < u32::from(cfg.drop_per_mille);
            if !drops {
                f.misses = f.misses.saturating_add(1);
                continue;
            }
            f.misses = 0;
        }
        let rarity = draw_rarity(
            &mut rng.loot,
            &if elite { up } else { tier }.weights,
            f.rarity,
        );
        let id = EntityId(*next_id);
        *next_id += 1;
        f.on_map.push(StarState { id, at, rarity });
        events.push(DomainEvent::StarDropped { star: id, rarity });
    }
}

/// The level of player `me`; 1 in a stage without levels.
fn level_of(s: &State, me: PlayerIdx) -> u32 {
    s.players
        .career(me)
        .progress
        .as_ref()
        .map_or(1, |p| p.level)
}

/// The highest level among the players that are up: what a drop on the shared map is drawn for
/// (ADR-SIM-7 point 5). When none is up (the tick the run fails), the highest among the present
/// ones, as targeting does.
fn top_level(s: &State) -> u32 {
    top_up_level(s).unwrap_or_else(|| {
        s.players
            .indices()
            .map(|me| level_of(s, me))
            .max()
            .unwrap_or(1)
    })
}

/// The drop table's tier for `level`.
pub(super) fn tier_of(cfg: &Stars, level: u32) -> Option<&Tier> {
    cfg.tiers
        .iter()
        .rev()
        .find(|t| t.from_level <= level)
        .or(cfg.tiers.first())
}

/// The highest level among the present players that are up; `None` when none is.
pub(super) fn top_up_level(s: &State) -> Option<u32> {
    s.players
        .indices()
        .filter(|&me| s.players[me].is_up())
        .map(|me| level_of(s, me))
        .max()
}

/// The boss died this tick: every present player that is up gets the boss's drops, at one tier
/// for all, the drop table's tier for the highest level among them. A downed or out player gets
/// none; nobody up, nothing is given (ADR-SIM-7 point 5). A solo player at 0 hit points is not up.
pub(super) fn boss_killed(stage: &World, s: &mut State) {
    super::starlight::boss_bonus(&stage.equipment_rules, s);
    super::drops::boss_killed(stage, s)
}

/// A rarity drawn by `weights`, leaving out those outside `range`, the difficulty's:
/// the lowest of the range when it leaves no weight. The generator is drawn once either way.
fn draw_rarity(rng: &mut SplitMix64, weights: &RarityWeights, range: (Rarity, Rarity)) -> Rarity {
    let weight = |rarity: Rarity| match (range.0..=range.1).contains(&rarity) {
        true => u32::from(weights.get(rarity)),
        false => 0,
    };
    let total: u32 = Rarity::ALL.into_iter().map(weight).sum();
    let mut left = rng.below(total.max(1));
    for rarity in Rarity::ALL {
        let w = weight(rarity);
        if left < w {
            return rarity;
        }
        left -= w;
    }
    range.0
}

/// The main character takes the stars inside its pickup circle; each gives it a drop roll.
pub(super) fn pick_up(stage: &World, s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    super::drops::pick_up(stage, s, me, events)
}

/// The run's sets and how far each is, for the snapshot.
pub(crate) fn constellation_views(
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
) -> Vec<ConstellationView> {
    let Some(o) = s.players.career(me).offers.as_ref() else {
        return Vec::new();
    };
    rewards
        .constellations()
        .sets()
        .iter()
        .zip(&o.set_counts)
        .map(|(c, held)| ConstellationView {
            id: c.id.clone(),
            name: c.name.clone(),
            held: *held,
            tier: c.tier_reached(*held),
            needed: c.needed(*held),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_content::RunContent;
    use crate::sim::build::Build;
    use crate::version::Versions;

    const ALL: (Rarity, Rarity) = (Rarity::Common, Rarity::Supernova);

    fn weights(common: u16, rare: u16, legendary: u16) -> RarityWeights {
        RarityWeights {
            common,
            rare,
            legendary,
            ..RarityWeights::default()
        }
    }

    #[test]
    fn a_range_leaves_out_the_rarities_outside_it() {
        let w = weights(50, 35, 15);
        let draws = |range| -> Vec<Rarity> {
            let mut rng = SplitMix64::new(9);
            (0..400).map(|_| draw_rarity(&mut rng, &w, range)).collect()
        };
        let all = draws(ALL);
        assert!(all.contains(&Rarity::Common) && all.contains(&Rarity::Legendary));
        let capped = draws((Rarity::Common, Rarity::Rare));
        assert!(capped.iter().all(|r| *r <= Rarity::Rare));
        assert!(capped.contains(&Rarity::Rare));
        let raised = draws((Rarity::Rare, Rarity::Supernova));
        assert!(raised.iter().all(|r| *r >= Rarity::Rare));
    }

    #[test]
    fn a_range_that_leaves_no_weight_gives_its_lowest_rarity() {
        let mut rng = SplitMix64::new(1);
        let only_common = weights(100, 0, 0);
        assert_eq!(
            draw_rarity(&mut rng, &only_common, (Rarity::Rare, Rarity::Supernova)),
            Rarity::Rare
        );
    }

    #[test]
    fn a_range_that_leaves_every_weight_in_changes_no_draw() {
        let w = weights(50, 35, 15);
        let (mut a, mut b) = (SplitMix64::new(3), SplitMix64::new(3));
        for _ in 0..100 {
            let wide = draw_rarity(&mut a, &w, ALL);
            let exact = draw_rarity(&mut b, &w, (Rarity::Common, Rarity::Legendary));
            assert_eq!(wide, exact);
        }
    }

    #[test]
    fn heal_per_kills_picks_add_their_amounts_and_heal_at_the_shortest_interval() {
        let heal = |id: &str, amount: u32, kills: u32| {
            serde_json::json!({ "id": id, "rarity": "epic", "named": true, "star": id,
                "limit": 1, "effects": [{ "effect": "heal_per_kills", "amount": amount,
                "kills": kills }] })
        };
        let costs = |rarity: &str, lumine_cost: u32| serde_json::json!({ "rarity": rarity, "lumine_cost": lumine_cost });
        let file = serde_json::json!({
            "rarities": [costs("common", 1), costs("uncommon", 2), costs("rare", 3),
                costs("epic", 4), costs("legendary", 6), costs("supernova", 10)],
            "rewards": [heal("a", 300, 10), heal("b", 200, 4)] });
        let rewards =
            Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("valid");
        let stage = super::super::World::for_test(RunContent::bundled().trial().clone());
        let one = Build::of(&stage, &rewards, &[1, 0]);
        assert_eq!((one.kill_heal, one.kill_heal_every), (300, 10));
        let both = Build::of(&stage, &rewards, &[1, 1]);
        assert_eq!((both.kill_heal, both.kill_heal_every), (500, 4));
    }
}
