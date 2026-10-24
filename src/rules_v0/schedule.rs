//! The tick order of `sim_version` 0, as data: `TICK` lists every step of a tick in the order
//! it runs, and `step` loops over it. `combat.md`'s tick order and the table in the module doc
//! of `rules_v0` name the same steps; `tick_doc_lists_the_schedule` keeps the table and
//! `TICK` equal.

use super::{
    World, agents, attacks, boss, collide, count, decide_outcome, entity, expertise, horde,
    loadout, move_bullets, open_offers, orbit, player, push, runes, skill, spawn_waves, zones,
};
use crate::engine::{Load, Outcome, State};
use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::input::Input;
use crate::rewards::Rewards;

/// What the steps of a tick read and write: the run's content, its state, the tick's input,
/// and what the steps pass on to each other within the tick.
pub(super) struct Ctx<'a> {
    pub(super) world: &'a World,
    pub(super) rewards: &'a Rewards,
    pub(super) expertise: &'a Expertise,
    pub(super) s: &'a mut State,
    pub(super) input: Input,
    pub(super) events: &'a mut Vec<DomainEvent>,
    pub(super) load: &'a mut Load,
    /// A status that deals damage over time is on someone: set by `statuses_and_aggro`, read by
    /// `collide`.
    pub(super) dots: bool,
}

impl<'a> Ctx<'a> {
    pub(super) fn new(
        world: &'a World,
        rewards: &'a Rewards,
        expertise: &'a Expertise,
        s: &'a mut State,
        input: Input,
        events: &'a mut Vec<DomainEvent>,
        load: &'a mut Load,
    ) -> Ctx<'a> {
        Ctx {
            world,
            rewards,
            expertise,
            s,
            input,
            events,
            load,
            dots: false,
        }
    }
}

/// One step of a tick.
pub(super) type Step = fn(&mut Ctx);

/// Every step of a tick, in order. A step that applies only sometimes tests its condition
/// itself. `spawn_boss` comes first: the boss whose time the tick reaches is there before the
/// tick counter advances.
pub(super) const TICK: &[(&str, Step)] = &[
    ("spawn_boss", |c| horde::spawn_boss(c.world, c.s)),
    ("advance_tick", |c| c.s.tick += 1),
    ("regen", |c| loadout::regen(c.s)),
    ("spawn_waves", |c| spawn_waves(c.world, c.s)),
    ("spawn_elite", |c| {
        horde::spawn_elite(c.world, c.s, c.events)
    }),
    ("statuses_and_aggro", |c| {
        c.dots = entity::tick_statuses_and_aggro(c.s, c.events);
    }),
    ("runes_tick", |c| runes::tick(c.s)),
    ("advance_runs", |c| {
        attacks::advance_runs(c.world, c.s, c.events)
    }),
    ("player_update", |c| {
        player::update(c.world, c.rewards, c.s, c.input, c.events);
    }),
    ("runes_fly", |c| {
        if !c.s.player.runes.is_empty() {
            runes::fly(c.world, c.s);
        }
    }),
    ("orbits", |c| orbit::act(c.world, c.rewards, c.s, c.events)),
    ("boss_act", |c| boss::act(c.world, c.s, c.events)),
    ("call_elites", |c| boss::call_elites(c.world, c.s, c.events)),
    ("agents_act", |c| agents::act(c.world, c.s, c.events)),
    ("push_apart", |c| push::apply(c.world, c.s)),
    ("skill_pull", |c| skill::pull(c.world, c.s)),
    ("runes_steer", |c| {
        if !c.s.player.runes.is_empty() {
            runes::steer(c.world, c.s);
        }
    }),
    ("zones_advance", |c| zones::advance(c.s)),
    ("fire_hooks", |c| attacks::fire_hooks(c.s)),
    ("run_attacks", |c| {
        let (tasks, emitter_steps) = attacks::run(c.world, c.s);
        *c.load = Load {
            enemies: count(&c.s.enemies),
            tasks,
            bullets: count(&c.s.bullets),
            zones: count(&c.s.zones),
            orbs: c.s.orbs,
            emitter_steps,
        };
    }),
    ("move_bullets", |c| move_bullets(c.world, c.s)),
    ("collide", |c| collide(c.world, c.s, c.dots, c.events)),
    ("level_up", |c| expertise::level_up(c.expertise, c.s)),
    ("decide_outcome", |c| decide_outcome(c.world, c.s, c.events)),
    ("open_offers", |c| match c.s.outcome {
        Outcome::Running | Outcome::Cleared => {
            open_offers(c.world, c.rewards, c.expertise, c.s, c.events);
        }
        Outcome::Failed => {}
    }),
];

#[cfg(test)]
mod tests {
    use super::TICK;

    /// The step names in the table between the `<!-- tick -->` markers of `rules_v0`'s module
    /// doc: the first code-quoted cell of each row.
    fn documented() -> Vec<String> {
        let src = include_str!("mod.rs");
        let (_, rest) = src.split_once("<!-- tick -->").expect("opening marker");
        let (table, _) = rest.split_once("<!-- /tick -->").expect("closing marker");
        table
            .lines()
            .filter_map(|l| l.trim_start_matches("//!").trim().strip_prefix('|'))
            .filter_map(|row| row.split('|').next())
            .filter_map(|cell| cell.trim().strip_prefix('`')?.strip_suffix('`'))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn tick_doc_lists_the_schedule() {
        let names: Vec<&str> = TICK.iter().map(|(name, _)| *name).collect();
        assert_eq!(documented(), names);
    }

    #[test]
    fn tick_names_are_unique_and_start_with_the_boss() {
        let mut names: Vec<&str> = TICK.iter().map(|(name, _)| *name).collect();
        assert_eq!(names[0], "spawn_boss");
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TICK.len());
    }
}
