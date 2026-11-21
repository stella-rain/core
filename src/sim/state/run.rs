//! The run's state, its generators and the echo (ADR-SIM-2, ADR-SIM-3).

use crate::attack::{DamageType, ZoneShape};
use crate::engine::Outcome;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::inline::InlineVec;
use crate::kinds::KindTable;
use crate::match_ups::MatchUps;
use crate::rng::{SplitMix64, TAG_ATTACK, TAG_HORDE, TAG_LOOT, TAG_SPAWN, derive};
use crate::runes::Runes;
use crate::sim::{AttackIndex, RangedPool};
use std::sync::Arc;

use super::{
    AgentState, AttackRun, AttackTask, BossState, Dir, HealPickup, Horde, MoveIndex, OrbState,
    Players, StarField, ZoneState,
};

/// Firings or casts left when there is no limit.
pub(crate) const UNLIMITED: u32 = u32::MAX;

/// The run's generators, one per purpose, so a change to one kind of draw leaves the others
/// where they were (ADR-SIM-2). Each is `rng::derive(seed, tag)`; the hash writes them in this
/// order. The offer generator is per player slot, in `Career::offer` (ADR-SIM-7): the hash and
/// savepoint format 1 write slot 0's between `loot` and `horde`, where it was.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Streams {
    /// The angle of a wave's ring spawn.
    pub spawn: SplitMix64,
    /// An attack's random numbers.
    pub attack: SplitMix64,
    /// Heal drops, star drops and a star's rarity.
    pub loot: SplitMix64,
    /// The horde's boss and elite draws.
    pub horde: SplitMix64,
}

impl Streams {
    pub fn new(seed: u64) -> Self {
        Self {
            spawn: derive(seed, TAG_SPAWN),
            attack: derive(seed, TAG_ATTACK),
            loot: derive(seed, TAG_LOOT),
            horde: derive(seed, TAG_HORDE),
        }
    }

    /// The four states, in hash order (the offer generator is each career's).
    #[cfg(test)]
    pub fn states(&self) -> [u64; 4] {
        [
            self.spawn.state(),
            self.attack.state(),
            self.loot.state(),
            self.horde.state(),
        ]
    }
}

/// The whole state of a run. Everything here is in the state hash (`StableHash`, in
/// `state_hash.rs`) except the fields that impl marks `_  // not hashed: <why>`: content, derived
/// values and buffers.
#[derive(Debug)]
pub(crate) struct State {
    /// Buffers reused from tick to tick; not part of the state (not hashed, not cloned).
    pub scratch: crate::sim::Scratch,
    /// The run's kinds, fixed at its start (core#271).
    pub kinds: Arc<KindTable>,
    pub tick: u32,
    pub rng: Streams,
    pub outcome: Outcome,
    /// The players, by slot (`Players::id` is a slot's entity number now): one now (ADR-SIM-7).
    pub players: Players,
    pub boss: Option<BossState>,
    /// The allies that act on their own, in entity-ID order.
    pub companions: Vec<AgentState>,
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<AgentState>,
    /// Attacks that are being carried out, oldest first (ADR-SIM-4 tier 3).
    pub tasks: Vec<AttackTask>,
    /// Phased attacks in progress, in the order they started.
    pub runs: Vec<AttackRun>,
    /// What `runs` leave each attacker's speed; updated wherever `runs` changes.
    pub move_index: MoveIndex,
    /// The ranged tokens cooling (RA1, DS83): the tick each is free on and its pool, sorted by
    /// both. At most `MAX_RANGED_ATTACKERS`: a pool grants no more tokens than the ones neither
    /// held nor cooling.
    pub ranged_cooling: Vec<(u32, RangedPool)>,
    /// In spawn order.
    pub orbs: Vec<OrbState>,
    /// Zones placed and not yet over, in the order they were placed (core Z1).
    pub zones: Vec<ZoneState>,
    /// Enemies spawned so far, per wave.
    pub spawned: Vec<u16>,
    /// The next entity number. It starts at `MAX_PLAYERS`: the numbers 0 to 2 are the player slots,
    /// present or not (ADR-SIM-7). The hash and the savepoint write it, like every entity number,
    /// as it is.
    pub next_id: u32,
    /// Cage groups taken so far (Z3a-3).
    pub next_orb_group: u32,
    /// Falling stars and offers; `None` in a stage without stars.
    pub stars: Option<StarField>,
    /// Heal pickups on the map, in the order they dropped; `None` in a stage without healing.
    pub heal_pickups: Option<Vec<HealPickup>>,
    /// What each match-up does to a hit: content, fixed for the run and covered by
    /// the content hash, so the state hash leaves it out.
    pub match_ups: MatchUps,
    /// What each on-hit rune does when it fires: content, as the match-ups are.
    pub runes: Arc<Runes>,
    /// The stun rules: content, as the match-ups are.
    pub stun: crate::run_stats::StunRules,
    /// The cap of the attack speed total (app#298): content, as the stun rules are.
    pub max_attack_speed_pct: u32,
    /// The damage an attack does when an enemy or the boss makes it and the attack names none:
    /// the run stats file's, content as the stun rules are.
    pub enemy_attack_damage: u32,
    /// The cap of Rune Haste (app#298): the rewards file's, held to the sum of the picks where
    /// it applies.
    pub max_rune_haste_pct: u32,
    /// The bosses of a `time_limit` run, as they come; `None` in any other run.
    pub horde: Option<Horde>,
    /// The percents the run's difficulty scales enemies' hit points and attacks' damage by
    ///: fixed for the run, part of the setup, and hashed with it.
    pub hp_pct: u32,
    pub damage_pct: u32,
    /// The run's seed, which the critical-hit rolls are hashed from: they draw nothing from
    /// `rng`, so a run without crit chance spends the same random numbers as before.
    pub crit_seed: u64,
    /// The echoes of last tick's hits, waiting for this tick's `collide`, in the order the hits
    /// landed; at most `MAX_ECHOES_PER_TICK`.
    pub echoes: Vec<Echo>,
    /// The repeats waiting to be dealt, at most `MAX_REPEATS_QUEUED`; empty until a rule queues
    /// one.
    pub repeats: Vec<Repeat>,
    /// The lingering areas, at most `MAX_LINGERS_ALIVE`; empty until a rule opens one.
    pub lingers: Vec<Linger>,
}

impl Clone for State {
    fn clone(&self) -> State {
        let State {
            scratch,
            kinds,
            tick,
            rng,
            outcome,
            players,
            boss,
            companions,
            enemies,
            tasks,
            runs,
            move_index,
            ranged_cooling,
            orbs,
            zones,
            spawned,
            next_id,
            next_orb_group,
            stars,
            heal_pickups,
            match_ups,
            runes,
            stun,
            max_attack_speed_pct,
            enemy_attack_damage,
            max_rune_haste_pct,
            horde,
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
            repeats,
            lingers,
        } = self;
        State {
            scratch: scratch.clone(),
            kinds: kinds.clone(),
            tick: *tick,
            rng: rng.clone(),
            outcome: *outcome,
            players: players.clone(),
            boss: boss.clone(),
            companions: companions.clone(),
            enemies: enemies.clone(),
            tasks: tasks.clone(),
            runs: runs.clone(),
            move_index: move_index.clone(),
            ranged_cooling: ranged_cooling.clone(),
            orbs: orbs.clone(),
            zones: zones.clone(),
            spawned: spawned.clone(),
            next_id: *next_id,
            next_orb_group: *next_orb_group,
            stars: stars.clone(),
            heal_pickups: heal_pickups.clone(),
            match_ups: *match_ups,
            runes: runes.clone(),
            stun: *stun,
            max_attack_speed_pct: *max_attack_speed_pct,
            enemy_attack_damage: *enemy_attack_damage,
            max_rune_haste_pct: *max_rune_haste_pct,
            horde: horde.clone(),
            hp_pct: *hp_pct,
            damage_pct: *damage_pct,
            crit_seed: *crit_seed,
            echoes: echoes.clone(),
            repeats: repeats.clone(),
            lingers: lingers.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &State) {
        let State {
            scratch,
            kinds,
            tick,
            rng,
            outcome,
            players,
            boss,
            companions,
            enemies,
            tasks,
            runs,
            move_index,
            ranged_cooling,
            orbs,
            zones,
            spawned,
            next_id,
            next_orb_group,
            stars,
            heal_pickups,
            match_ups,
            runes,
            stun,
            max_attack_speed_pct,
            enemy_attack_damage,
            max_rune_haste_pct,
            horde,
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
            repeats,
            lingers,
        } = self;
        let State {
            scratch: s_scratch,
            kinds: s_kinds,
            tick: s_tick,
            rng: s_rng,
            outcome: s_outcome,
            players: s_players,
            boss: s_boss,
            companions: s_companions,
            enemies: s_enemies,
            tasks: s_tasks,
            runs: s_runs,
            move_index: s_move_index,
            ranged_cooling: s_ranged_cooling,
            orbs: s_orbs,
            zones: s_zones,
            spawned: s_spawned,
            next_id: s_next_id,
            next_orb_group: s_next_orb_group,
            stars: s_stars,
            heal_pickups: s_heal_pickups,
            match_ups: s_match_ups,
            runes: s_runes,
            stun: s_stun,
            max_attack_speed_pct: s_max_attack_speed_pct,
            enemy_attack_damage: s_enemy_attack_damage,
            max_rune_haste_pct: s_max_rune_haste_pct,
            horde: s_horde,
            hp_pct: s_hp_pct,
            damage_pct: s_damage_pct,
            crit_seed: s_crit_seed,
            echoes: s_echoes,
            repeats: s_repeats,
            lingers: s_lingers,
        } = source;
        scratch.clone_from(s_scratch);
        kinds.clone_from(s_kinds);
        *tick = *s_tick;
        rng.clone_from(s_rng);
        *outcome = *s_outcome;
        players.clone_from(s_players);
        boss.clone_from(s_boss);
        companions.clone_from(s_companions);
        enemies.clone_from(s_enemies);
        tasks.clone_from(s_tasks);
        runs.clone_from(s_runs);
        move_index.clone_from(s_move_index);
        ranged_cooling.clone_from(s_ranged_cooling);
        orbs.clone_from(s_orbs);
        zones.clone_from(s_zones);
        spawned.clone_from(s_spawned);
        *next_id = *s_next_id;
        *next_orb_group = *s_next_orb_group;
        stars.clone_from(s_stars);
        heal_pickups.clone_from(s_heal_pickups);
        *match_ups = *s_match_ups;
        runes.clone_from(s_runes);
        *stun = *s_stun;
        *max_attack_speed_pct = *s_max_attack_speed_pct;
        *enemy_attack_damage = *s_enemy_attack_damage;
        *max_rune_haste_pct = *s_max_rune_haste_pct;
        horde.clone_from(s_horde);
        *hp_pct = *s_hp_pct;
        *damage_pct = *s_damage_pct;
        *crit_seed = *s_crit_seed;
        echoes.clone_from(s_echoes);
        repeats.clone_from(s_repeats);
        lingers.clone_from(s_lingers);
    }
}

/// A repeat of a step's damage, queued to be dealt at its tick (ADR-SIM-5 point 9: the queue and
/// each entry are capped at decode). `Copy`: the hit list is inline, so an entry allocates nothing.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Repeat {
    /// The slot of the player whose step it repeats.
    pub owner: u8,
    /// The tick it is dealt on.
    pub due: u32,
    /// The part of the step's damage it deals, in percent.
    pub percent: u16,
    /// The chance, in percent, that a hit of the repeat fires a ready on-hit rune; none: every
    /// ready rune fires, no draw.
    pub proc_pct: Option<u16>,
    /// What it hits.
    pub anchor: RepeatAnchor,
    /// The attack whose phase `phase` was the step's last `active` one: its hit box and damage
    /// type are read from `world.attacks[attack].phases[phase]`.
    pub attack: AttackIndex,
    /// The index of that phase in the attack.
    pub phase: u8,
    /// The damage of that hit per target before `percent`, with the attack-up and the type bonus
    /// the step had when it started.
    pub damage: u32,
    /// The range the run had, in percent (at most `MAX_RANGE_PCT`), widening a `Place` hit box.
    pub width_pct: u32,
    /// The row of `World::repeat_bleeds` (counted from 1) whose stacks the repeat puts on what it
    /// hits and leaves standing; 0: none. Fixed when the repeat is queued, as `damage` is, so a
    /// style change inside the delay changes nothing. Hashed and saved.
    pub bleed: u8,
    /// 1 once the throw this entry belongs to has given its stack, 0 before; shared by the entries
    /// of one throw, which `owner` and `queued` name. Hashed and saved.
    pub stack_given: u8,
    /// The tick the throw's entries were queued on (at most `due`): with `owner` it names the
    /// throw, so the entries of one throw find each other. Hashed and saved.
    pub queued: u32,
}

/// A lingering area (ADR-SIM-5 point 9: the queue and each field are capped at decode; the queue is
/// held below `MAX_ZONES_ALIVE`, which the placement check will count it inside). It pulses every `rate_ticks` until `ticks_left` runs out. Every
/// number is resolved when the area opens (a style's area bonus is baked into `shape` and
/// `proc_pct`, as `Repeat` fixes `damage` and `width_pct` when queued), so a later style change
/// changes nothing inside it. Hashed and saved; the lingering rules are not here yet.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Linger {
    /// The slot of the player whose area it is.
    pub owner: u8,
    /// Where it stands.
    pub anchor: LingerAnchor,
    /// Ticks left, and the whole length (the view draws a fill against it).
    pub ticks_left: u16,
    pub ticks: u16,
    /// Ticks to the next pulse, and the rate (at least 1).
    pub next_pulse: u16,
    pub rate_ticks: u16,
    /// The opening step's damage per target, and the percent of it a pulse deals.
    pub damage: u32,
    pub damage_pct: u16,
    pub shape: ZoneShape,
    /// The pull's distance in px; 0: none.
    pub pull: Fx,
    pub cuts_orbs: bool,
    /// The chance, in percent, that a pulse fires a ready on-hit rune; none: no proc.
    pub proc_pct: Option<u16>,
}

/// Where a lingering area stands: on a place, or on its owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LingerAnchor {
    Place { at: Point, facing: Dir },
    Player,
}

/// Where a repeat lands: on the enemies the step hit, or on a place.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum RepeatAnchor {
    HitList(InlineVec<EntityId, { crate::limits::MAX_REPEAT_TARGETS }>),
    Place { at: Point, facing: Dir },
}

/// The unused places of an inline hit list hold it; they are never read.
impl Default for EntityId {
    fn default() -> Self {
        EntityId(0)
    }
}

/// One hit's echo, queued when the hit lands and dealt a tick later (core#NS2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Echo {
    pub source: EntityId,
    pub target: EntityId,
    /// Hit points taken, already the hit's percent of its damage.
    pub damage: u32,
    /// The type of the hit it echoes, which the echo's `Hit` event reports.
    pub damage_type: Option<DamageType>,
}
