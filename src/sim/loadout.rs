//! What a run's setup brings in at tick 0: the perk set's
//! effects, the auto combo, the rune pool, the carried stars and the starting rune. They apply
//! before the first step, in this order: the perk set, the carried stars in setup order, as if
//! each were picked from an offer (they count toward their reward's stack limit), then the
//! starting rune, then the Lumine star's slot (ADR-SAVE-7 point 4: Lv1, nothing drawn; the star
//! is in the build from tick 0, its stats, skill, orbit and set count included). Nothing is
//! drawn for any of them, so the generator's order is unchanged.
//! Regen, the auto combo and the rune pool are fixed for the run and part of the setup, which
//! the replay stores: the state hash writes nothing for them. The Lumine slot is career state
//! and is hashed with the equipment.

use super::World;
use crate::perk_sets::{PerkCategory, PerkEffect, PerkEntry};
use crate::rewards::{EquipmentRules, Rewards};
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::rune::{MAX_RUNES, Rune};
use crate::sim::equipment::{EquipSlot, Held, SlotKind};
use crate::sim::state::PlayerIdx;
use crate::sim::state::{HeldRune, State};

use super::{levels, runes, slots, stars};

/// The ticks between two regenerations: a second.
pub(super) const REGEN_TICKS: u32 = 60;

/// A validated setup's grants, resolved against the run's content.
pub(crate) struct Loadout {
    perk: Vec<PerkEntry>,
    rune_pool: u8,
    /// The carried stars, as places in the rewards file.
    carried: Vec<u16>,
    starting_rune: Option<Rune>,
    /// The Lumine star, as a place in the rewards file.
    lumine: Option<u16>,
}

impl Loadout {
    /// The setup's drop weight in percent for each category of `rules`, in the rules' order: the
    /// sum of the set's `drop_weight` amounts naming the category. Empty when the set has none.
    pub(crate) fn drop_weight_pct(&self, rules: &EquipmentRules) -> Vec<u32> {
        let weighs = |e: &&PerkEntry| e.effect == PerkEffect::DropWeight;
        if !self.perk.iter().any(|e| weighs(&e)) {
            return Vec::new();
        }
        rules
            .categories
            .iter()
            .map(|c| {
                self.perk
                    .iter()
                    .filter(weighs)
                    .filter(|e| {
                        e.category.as_ref().map(PerkCategory::as_str) == Some(c.id.as_str())
                    })
                    .map(|e| e.amount)
                    .sum()
            })
            .collect()
    }

    /// The grants of `setup`, which `run_setup::validate_in` accepted against `content`.
    pub(crate) fn of(setup: &RunSetup, content: &RunContent) -> Loadout {
        let perk = content
            .perk_sets()
            .index_of(&setup.perk_set)
            .map(|at| content.perk_sets().sets()[at].effects.clone())
            .unwrap_or_default();
        Loadout {
            perk,
            rune_pool: runes::pool_mask(&setup.rune_pool),
            carried: setup
                .carried
                .iter()
                .filter_map(|c| content.rewards().index_of(&c.reward))
                .map(|at| u16::try_from(at).unwrap_or(u16::MAX))
                .collect(),
            starting_rune: setup
                .starting_rune
                .as_ref()
                .and_then(|id| Rune::from_id(id.as_str())),
            lumine: setup
                .lumine_star
                .as_ref()
                .and_then(|id| content.rewards().index_of(id))
                .map(|at| u16::try_from(at).unwrap_or(u16::MAX)),
        }
    }
}

/// Applies `loadout` to the state before the first tick.
pub(crate) fn apply(
    stage: &World,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    loadout: &Loadout,
) {
    if let Some(f) = s.stars.as_mut() {
        f.rune_pool = loadout.rune_pool;
    }
    for entry in &loadout.perk {
        perk(s, me, *entry);
    }
    let State { stars, players, .. } = s;
    let (player, career) = players.both_mut(me);
    if let (Some(f), Some(o)) = (stars.as_ref(), career.offers.as_mut()) {
        for &i in &loadout.carried {
            stars::take(
                stage,
                rewards,
                &mut career.offer,
                &mut career.progress,
                player,
                &mut career.regen,
                f,
                o,
                i,
            );
        }
    }
    career.equipment.slots = slots::fresh_slots(&stage.equipment_rules);
    if let Some(rune) = loadout.starting_rune
        && player.runes.len() < MAX_RUNES
        && !player.runes.iter().any(|h| h.rune == rune)
    {
        player.runes.push(HeldRune {
            rune,
            cooldown_left: 0,
        });
        // The starting rune fills rune slot 0 (the slot list is empty in an offers run).
        if let Some(slot) = career
            .equipment
            .slots
            .get_mut(slots::first_rune_slot(&stage.equipment_rules))
        {
            slot.held = Some(Held::Rune(rune));
            slot.level = 1;
        }
    }
    if let Some(place) = loadout.lumine {
        career.equipment.lumine = Some(EquipSlot {
            kind: SlotKind::Lumine,
            held: Some(Held::Star(place)),
            level: 1,
            expertise: 0,
        });
    }
    // The Lumine star's effects are in the build from tick 0 (before = the build of nothing).
    slots::refresh(stage, rewards, player, career, &[], slots::Arrival::Start);
}

/// Adds one perk entry. The extra rerolls and banishes are stats of the player's `Progress`
/// (nothing without one).
pub(super) fn perk(s: &mut State, me: PlayerIdx, entry: PerkEntry) {
    let amount = entry.amount;
    match entry.effect {
        PerkEffect::Regen => {
            let career = s.players.career_mut(me);
            career.regen = career.regen.saturating_add(amount);
        }
        PerkEffect::ExtraRerolls | PerkEffect::ExtraBanishes => {
            let (player, career) = s.players.both_mut(me);
            if let (Some(stat), Some(p)) = (entry.effect.growth_stat(), career.progress.as_mut()) {
                levels::add_effect(p, player, stat, amount);
            }
        }
        effect => {
            let (player, career) = s.players.both_mut(me);
            if let (Some(effect), Some(p)) = (effect.stat(), career.progress.as_mut()) {
                levels::add_effect(p, player, effect, amount);
            }
        }
    }
}

/// A second has passed: the main character gets its regeneration back, never beyond the most.
pub(super) fn regen(s: &mut State, me: PlayerIdx) {
    let regen = s.players.career(me).regen;
    // Regeneration on a downed player would raise it from 0 and revive it.
    if regen > 0 && s.players[me].is_up() && s.tick % REGEN_TICKS == 0 {
        s.players[me].hp = s.players[me]
            .hp
            .saturating_add(regen)
            .min(s.players[me].max_hp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_fixtures::worst_case;
    use crate::engine::{Engine, Run};
    use crate::hash::{StableHash, StateHasher};
    use crate::sim::equipment::{EquipSlot, Held, SlotKind};
    use crate::sim::state::Players;

    fn start() -> (World, State) {
        let e = Engine::new_with(&worst_case::stage(), &worst_case::content()).expect("starts");
        match e.run() {
            Run::V2 { world, state } => (World::clone(world), state.clone()),
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    fn hash_of(s: &State) -> u64 {
        let mut h = StateHasher::new();
        s.stable_hash(&mut h);
        h.finish()
    }

    /// The state after `apply` of a setup with `lumine_star`, and the place of that star.
    fn applied(star: Option<usize>) -> (State, Option<u16>) {
        let content = worst_case::content();
        let (world, mut s) = start();
        let setup = RunSetup {
            lumine_star: star.map(|i| content.rewards().rewards()[i].id.clone()),
            ..RunSetup::default()
        };
        let loadout = Loadout::of(&setup, &content);
        apply(&world, content.rewards(), &mut s, PlayerIdx::SOLO, &loadout);
        (s, star.map(|i| u16::try_from(i).unwrap()))
    }

    fn lumine_of(place: u16) -> EquipSlot {
        EquipSlot {
            kind: SlotKind::Lumine,
            held: Some(Held::Star(place)),
            level: 1,
            expertise: 0,
        }
    }

    #[test]
    fn the_lumine_star_is_equipped_at_level_one_at_tick_zero() {
        let (none, _) = applied(None);
        assert!(
            none.players
                .career(PlayerIdx::SOLO)
                .equipment
                .lumine
                .is_none()
        );
        let (s, place) = applied(Some(1));
        let c = s.players.career(PlayerIdx::SOLO);
        assert_eq!(c.equipment.lumine, Some(lumine_of(place.unwrap())));
        // Nothing is drawn for it.
        assert_eq!(
            c.offer.state(),
            none.players.career(PlayerIdx::SOLO).offer.state()
        );
        assert_eq!(s.rng.loot.state(), none.rng.loot.state());
        assert_ne!(hash_of(&s), hash_of(&none), "the slot is hashed");

        // A party built from it holds the slot in all three slots.
        let party = Players::party(3, &s.players, s.crit_seed);
        for me in party.indices() {
            assert_eq!(party.career(me).equipment.lumine, Some(lumine_of(1)));
        }
        // A rejoiner gets it at level 1.
        let mut p = Players::party(2, &s.players, s.crit_seed);
        let template = s.players.template();
        let slot2 = PlayerIdx::new(2).unwrap();
        p.join(slot2, &template, s.crit_seed);
        assert_eq!(p.career(slot2).equipment.lumine, Some(lumine_of(1)));
    }
}
