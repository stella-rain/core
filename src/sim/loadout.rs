//! What a run's setup brings in at tick 0 (ADR-105 point 3, ADR-107 point 3): the perk set's
//! effects, the auto combo, the rune pool, the carried stars and the starting rune. They apply
//! before the first step, in this order: the perk set, the carried stars in setup order, as if
//! each were picked from an offer (they count toward their reward's stack limit), then the
//! starting rune. Nothing is drawn for any of them, so the generator's order is unchanged.
//! Regen, the auto combo and the rune pool are fixed for the run and part of the setup, which
//! the replay stores: the state hash writes nothing for them.

use super::World;
use crate::perk_sets::{PerkEffect, PerkEntry};
use crate::rewards::Rewards;
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::rune::{MAX_RUNES, Rune};
use crate::sim::state::PlayerIdx;
use crate::sim::state::{HeldRune, State};

use super::{levels, runes, stars};

/// The ticks between two regenerations: a second.
pub(super) const REGEN_TICKS: u32 = 60;

/// A validated setup's grants, resolved against the run's content.
pub(crate) struct Loadout {
    perk: Vec<PerkEntry>,
    rune_pool: u8,
    /// The carried stars, as places in the rewards file.
    carried: Vec<u16>,
    starting_rune: Option<Rune>,
}

impl Loadout {
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
        }
    }
}

/// Applies `loadout` to the state before the first tick.
pub(crate) fn apply(stage: &World, rewards: &Rewards, s: &mut State, loadout: &Loadout) {
    if let Some(f) = s.stars.as_mut() {
        f.rune_pool = loadout.rune_pool;
    }
    for entry in &loadout.perk {
        perk(s, *entry);
    }
    let State {
        rng,
        stars,
        progress,
        players,
        regen,
        ..
    } = s;
    let player = &mut players[PlayerIdx::SOLO];
    if let Some(f) = stars.as_mut() {
        for &i in &loadout.carried {
            stars::take(
                stage,
                rewards,
                &mut rng.offer,
                progress,
                player,
                regen,
                f,
                i,
            );
        }
    }
    if let Some(rune) = loadout.starting_rune
        && player.runes.len() < MAX_RUNES
        && !player.runes.iter().any(|h| h.rune == rune)
    {
        player.runes.push(HeldRune {
            rune,
            cooldown_left: 0,
        });
    }
}

fn perk(s: &mut State, entry: PerkEntry) {
    let amount = entry.amount;
    match entry.effect {
        PerkEffect::Regen => s.regen = s.regen.saturating_add(amount),
        PerkEffect::ExtraRerolls => {
            if let Some(f) = s.stars.as_mut() {
                f.rerolls_left = f.rerolls_left.saturating_add(amount);
            }
        }
        PerkEffect::ExtraBanishes => {
            if let Some(f) = s.stars.as_mut() {
                f.banishes_left = f.banishes_left.saturating_add(amount);
            }
        }
        effect => {
            if let (Some(effect), Some(p)) = (effect.stat(), s.progress.as_mut()) {
                levels::add_effect(p, &mut s.players[PlayerIdx::SOLO], effect, amount);
            }
        }
    }
}

/// A second has passed: the main character gets its regeneration back, never beyond the most.
pub(super) fn regen(s: &mut State) {
    if s.regen > 0 && s.tick % REGEN_TICKS == 0 {
        s.players[PlayerIdx::SOLO].hp = s.players[PlayerIdx::SOLO]
            .hp
            .saturating_add(s.regen)
            .min(s.players[PlayerIdx::SOLO].max_hp);
    }
}
