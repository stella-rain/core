//! The tick order of `sim_version` 1, as data: `TICK` lists every step of a tick in the order
//! it runs, and `step` loops over it. `combat.md`'s tick order and the table in the module doc
//! of `sim` name the same steps; `tick_doc_lists_the_schedule` keeps the table and
//! `TICK` equal. Each step is a named `#[inline(never)]` function, so a profile names it (DS71-1).

use super::{
    World, agents, attacks, boss, collide, count, dash, decide_outcome, entity, expertise, horde,
    leash, loadout, move_orbs, open_offers, orbit, player, push, runes, skill, spawn_waves, zones,
};
use crate::engine::{Load, Outcome};
use crate::event::DomainEvent;
use crate::expertise::Expertise;
use crate::input::Input;
use crate::rewards::Rewards;
use crate::sim::state::State;
use crate::sim::state::{MAX_PLAYERS, PlayerIdx};

/// What the steps of a tick read and write: the run's content, its state, the tick's inputs (one
/// per slot), and what the steps pass on to each other within the tick.
pub(super) struct Ctx<'a> {
    pub(super) world: &'a World,
    pub(super) rewards: &'a Rewards,
    pub(super) expertise: &'a Expertise,
    pub(super) s: &'a mut State,
    /// One input per slot, present or not, indexed by `PlayerIdx`.
    pub(super) inputs: &'a [Input; MAX_PLAYERS as usize],
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
        inputs: &'a [Input; MAX_PLAYERS as usize],
        events: &'a mut Vec<DomainEvent>,
        load: &'a mut Load,
    ) -> Ctx<'a> {
        Ctx {
            world,
            rewards,
            expertise,
            s,
            inputs,
            events,
            load,
            dots: false,
        }
    }

    /// The input of slot `me`, the player the per-player steps act for.
    pub(super) fn input(&self, me: PlayerIdx) -> Input {
        self.inputs[me.get()]
    }
}

/// Some present player holds a rune. A party of one tests slot 0 alone.
#[inline]
fn holds_rune(s: &State) -> bool {
    if s.players.is_solo() {
        return !s.players[PlayerIdx::SOLO].runes.is_empty();
    }
    s.players.iter().any(|p| !p.runes.is_empty())
}

/// One step of a tick.
pub(super) type Step = fn(&mut Ctx);

/// The steps of a tick, one named function each, so a profile names the step (DS71-1).
pub(super) mod steps {
    use super::*;

    #[inline(never)]
    pub(super) fn spawn_boss(c: &mut Ctx) {
        horde::spawn_boss(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn advance_tick(c: &mut Ctx) {
        c.s.tick += 1;
    }

    #[inline(never)]
    pub(super) fn regen(c: &mut Ctx) {
        if c.s.players.is_solo() {
            return loadout::regen(c.s, PlayerIdx::SOLO);
        }
        for me in c.s.players.indices() {
            loadout::regen(c.s, me);
        }
    }

    #[inline(never)]
    pub(super) fn spawn_waves(c: &mut Ctx) {
        super::spawn_waves(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn spawn_elite(c: &mut Ctx) {
        horde::spawn_elite(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn leash(c: &mut Ctx) {
        c.load.relocated = leash::relocate(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn statuses_and_aggro(c: &mut Ctx) {
        c.dots = entity::tick_statuses_and_aggro(c.s, c.world.tuning.aggro_decay, c.events);
    }

    #[inline(never)]
    pub(super) fn runes_tick(c: &mut Ctx) {
        if c.s.players.is_solo() {
            return runes::tick(c.s, PlayerIdx::SOLO);
        }
        for me in c.s.players.indices() {
            runes::tick(c.s, me);
        }
    }

    #[inline(never)]
    pub(super) fn advance_runs(c: &mut Ctx) {
        attacks::advance_runs(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn tether(c: &mut Ctx) {
        if c.s.players.is_solo() || c.s.players.up_mask().count_ones() < 2 {
            return;
        }
        player::tether(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn player_update(c: &mut Ctx) {
        crate::sim::tick_down_and_revive(c.world, c.s, c.inputs, c.events);
        if c.s.players.is_solo() {
            let input = c.input(PlayerIdx::SOLO);
            return player::update(
                c.world,
                c.rewards,
                c.s,
                PlayerIdx::SOLO,
                input,
                false,
                c.events,
            );
        }
        // Slot order, each player's whole update before the next one's (ADR-SIM-7 point 6).
        for me in c.s.players.indices() {
            let input = c.input(me);
            // After `tick_down_and_revive`: a revive finished this tick frees its reviver.
            let rooted = crate::sim::near_downed(c.s, c.inputs, me);
            player::update(c.world, c.rewards, c.s, me, input, rooted, c.events);
        }
    }

    #[inline(never)]
    pub(super) fn runes_fly(c: &mut Ctx) {
        if holds_rune(c.s) {
            runes::fly(c.world, c.s);
        }
    }

    #[inline(never)]
    pub(super) fn orbits(c: &mut Ctx) {
        if c.s.players.is_solo() {
            return orbit::act(c.world, c.rewards, c.s, PlayerIdx::SOLO, c.events);
        }
        for me in c.s.players.indices() {
            // A downed or out player's orbs do not attack.
            if c.s.players[me].is_up() {
                orbit::act(c.world, c.rewards, c.s, me, c.events);
            }
        }
    }

    #[inline(never)]
    pub(super) fn boss_act(c: &mut Ctx) {
        boss::act(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn call_elites(c: &mut Ctx) {
        boss::call_elites(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn agents_act(c: &mut Ctx) {
        agents::act(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn dash_attackers(c: &mut Ctx) {
        dash::act(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn push_apart(c: &mut Ctx) {
        push::apply(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn skill_pull(c: &mut Ctx) {
        if c.s.players.is_solo() {
            return skill::pull(c.world, c.s, PlayerIdx::SOLO);
        }
        for me in c.s.players.indices() {
            skill::pull(c.world, c.s, me);
        }
    }

    #[inline(never)]
    pub(super) fn runes_steer(c: &mut Ctx) {
        if holds_rune(c.s) {
            runes::steer(c.world, c.s);
        }
    }

    #[inline(never)]
    pub(super) fn zones_advance(c: &mut Ctx) {
        zones::advance(c.s);
    }

    #[inline(never)]
    pub(super) fn fire_hooks(c: &mut Ctx) {
        attacks::fire_hooks(c.s);
    }

    #[inline(never)]
    pub(super) fn run_attacks(c: &mut Ctx) {
        let (tasks, emitter_steps) = attacks::run(c.world, c.s);
        c.load.enemies = count(&c.s.enemies);
        c.load.tasks = tasks;
        c.load.zones = count(&c.s.zones);
        c.load.orbs = count(&c.s.orbs);
        c.load.emitter_steps = emitter_steps;
    }

    #[inline(never)]
    pub(super) fn move_orbs(c: &mut Ctx) {
        super::move_orbs(c.world, c.s);
    }

    #[inline(never)]
    pub(super) fn collide(c: &mut Ctx) {
        super::collide(c.world, c.s, c.dots, c.events);
        // The kills `collide` counted give the killers' star slots expertise (equipment mode).
        if c.world.equipment_mode {
            crate::sim::slots::credit_kills(c.world, c.rewards, c.s);
        }
    }

    #[inline(never)]
    pub(super) fn level_up(c: &mut Ctx) {
        for me in c.s.players.indices() {
            expertise::level_up(c.expertise, c.s, me);
        }
    }

    #[inline(never)]
    pub(super) fn decide_outcome(c: &mut Ctx) {
        super::decide_outcome(c.world, c.s, c.events);
    }

    #[inline(never)]
    pub(super) fn open_offers(c: &mut Ctx) {
        match c.s.outcome {
            Outcome::Running | Outcome::Cleared => {
                crate::sim::pick_due(c.world, c.rewards, c.expertise, c.s, c.events);
                super::open_offers(c.world, c.rewards, c.expertise, c.s, c.events);
            }
            Outcome::Failed => {}
        }
    }
}

/// Pairs each step function with its own name.
macro_rules! steps {
    ($($name:ident),* $(,)?) => {
        &[$((stringify!($name), steps::$name as Step)),*]
    };
}

/// Every step of a tick, in order. A step that applies only sometimes tests its condition
/// itself. `spawn_boss` comes first: the boss whose time the tick reaches is there before the
/// tick counter advances.
pub(super) const TICK: &[(&str, Step)] = steps![
    spawn_boss,
    advance_tick,
    regen,
    spawn_waves,
    spawn_elite,
    leash,
    statuses_and_aggro,
    runes_tick,
    advance_runs,
    tether,
    player_update,
    runes_fly,
    orbits,
    boss_act,
    call_elites,
    agents_act,
    dash_attackers,
    push_apart,
    skill_pull,
    runes_steer,
    zones_advance,
    fire_hooks,
    run_attacks,
    move_orbs,
    collide,
    level_up,
    decide_outcome,
    open_offers,
];

#[cfg(test)]
mod tests {
    use super::TICK;

    /// The step names in the table between the `<!-- tick -->` markers of `sim`'s module
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

    /// A fn pointer carries no name, so the source is read: `TICK` is built by `steps![..]` from
    /// the names in order, and each is a `#[inline(never)]` function (DS71-1).
    #[test]
    fn every_step_is_a_named_function() {
        let src = include_str!("schedule.rs");
        let (_, rest) = src.split_once("const TICK").expect("TICK");
        let (_, rest) = rest.split_once("steps![").expect("steps![ opens the list");
        let (list, _) = rest.split_once("];").expect("];  closes the list");
        let listed: Vec<&str> = list
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .collect();
        let names: Vec<&str> = TICK.iter().map(|(name, _)| *name).collect();
        assert_eq!(listed, names);
        for name in names {
            let sig = format!("fn {name}(c: &mut Ctx)");
            let (before, _) = src.split_once(&sig).unwrap_or_else(|| panic!("no {sig}"));
            let before = before.trim_end();
            let before = before
                .strip_suffix("pub(super)")
                .unwrap_or(before)
                .trim_end();
            assert!(
                before.ends_with("#[inline(never)]"),
                "{name} is not #[inline(never)]"
            );
        }
    }
}
