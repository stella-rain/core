//! Every `State` type: the whole state of a run, hashed by `StableHash` (`state_hash.rs`, ADR-SIM-3).

use crate::attack::{DamageType, ZoneShape};
use crate::behaviour::BossCall;
use crate::engine::Outcome;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::inline::InlineVec;
use crate::kinds::{KindIndex, KindTable};
use crate::match_ups::MatchUps;
use crate::rewards::Rarity;
use crate::rng::{SplitMix64, TAG_ATTACK, TAG_HORDE, TAG_LOOT, TAG_OFFER, TAG_SPAWN, derive};
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::runes::Runes;
use crate::sim::{AttackIndex, BossIndex, DefIndex, ProgramIndex, RangedPool, World};
use crate::snapshot::{EntityKind, ExpertiseOfferKind, OfferKind};
use crate::validate::{self};
use std::sync::LazyLock;

/// Firings or casts left when there is no limit.
pub(crate) const UNLIMITED: u32 = u32::MAX;

/// The run's generators, one per purpose, so a change to one kind of draw leaves the others
/// where they were (ADR-SIM-2). Each is `rng::derive(seed, tag)`; the hash writes them in this
/// order.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Streams {
    /// The angle of a wave's ring spawn.
    pub spawn: SplitMix64,
    /// An attack's random numbers.
    pub attack: SplitMix64,
    /// Heal drops, star drops and a star's rarity.
    pub loot: SplitMix64,
    /// The star, expertise and rune offers.
    pub offer: SplitMix64,
    /// The horde's boss and elite draws.
    pub horde: SplitMix64,
}

impl Streams {
    pub fn new(seed: u64) -> Self {
        Self {
            spawn: derive(seed, TAG_SPAWN),
            attack: derive(seed, TAG_ATTACK),
            loot: derive(seed, TAG_LOOT),
            offer: derive(seed, TAG_OFFER),
            horde: derive(seed, TAG_HORDE),
        }
    }

    /// The five states, in hash order.
    #[cfg(test)]
    pub fn states(&self) -> [u64; 5] {
        [
            self.spawn.state(),
            self.attack.state(),
            self.loot.state(),
            self.offer.state(),
            self.horde.state(),
        ]
    }
}

/// The whole state of a run. Everything here is in the state hash (`StableHash`, in
/// `state_hash.rs`) except the fields that impl marks `_  // not hashed: <why>`: content, derived
/// values and buffers.
#[derive(Clone, Debug)]
pub(crate) struct State {
    /// Buffers reused from tick to tick; not part of the state (not hashed, not cloned).
    pub scratch: crate::sim::Scratch,
    /// The run's kinds, fixed at its start (core#271).
    pub kinds: KindTable,
    pub tick: u32,
    pub rng: Streams,
    pub outcome: Outcome,
    /// The players, in entity-ID order (`PlayerIdx::id`): one now (ADR-SIM-12).
    pub players: Players,
    pub boss: Option<BossState>,
    /// The allies that act on their own, in entity-ID order.
    pub companions: Vec<AgentState>,
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<AgentState>,
    /// Attacks that are being carried out, oldest first (ADR-036 tier 3).
    pub tasks: Vec<AttackTask>,
    /// Phased attacks in progress, in the order they started (ADR-055).
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
    pub next_id: u32,
    /// Cage groups taken so far (Z3a-3).
    pub next_orb_group: u32,
    /// Experience and levels; `None` in a stage without levels.
    pub progress: Option<Progress>,
    /// Falling stars and offers; `None` in a stage without stars.
    pub stars: Option<StarField>,
    /// Heal pickups on the map, in the order they dropped; `None` in a stage without healing.
    pub heal_pickups: Option<Vec<HealPickup>>,
    /// The expertise gauge; `None` in a stage whose enemies and boss give no expertise points.
    pub expertise: Option<ExpertiseGauge>,
    /// What each match-up does to a hit (ADR-100): content, fixed for the run and covered by
    /// the content hash, so the state hash leaves it out.
    pub match_ups: MatchUps,
    /// What each on-hit rune does when it fires (ADR-099): content, as the match-ups are.
    pub runes: Runes,
    /// The stun rules (ADR-085 points 4 and 6): content, as the match-ups are.
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
    /// The Stardust the run has earned, times 10 (ADR-109): `Some` in a stage with a Stardust
    /// factor.
    pub stardust: Option<u32>,
    /// Hit points the main character regenerates each second (a perk, ADR-105): set at the start
    /// and raised by rewards.
    pub regen: u32,
    /// The percents the run's difficulty scales enemies' hit points and attacks' damage by
    /// (ADR-110): fixed for the run, part of the setup, and hashed with it.
    pub hp_pct: u32,
    pub damage_pct: u32,
    /// The run's seed, which the critical-hit rolls are hashed from: they draw nothing from
    /// `rng`, so a run without crit chance spends the same random numbers as before.
    pub crit_seed: u64,
    /// The echoes of last tick's hits, waiting for this tick's `collide`, in the order the hits
    /// landed; at most `MAX_ECHOES_PER_TICK`.
    pub echoes: Vec<Echo>,
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

/// How far a `time_limit` run's bosses have come (core#212): the times, and which pool entries
/// are used up. What the bosses are is `World::horde` and `World::bosses`, fixed for the run.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Horde {
    pub boss_ticks: Vec<u32>,
    /// The boss times at which the boss calls no elites (core#336).
    pub no_call_ticks: Vec<u32>,
    /// The mid-bosses' hit points by slot, in percent (core#336).
    pub mid_boss_hp_pct: Vec<u32>,
    /// Bosses that have come, mid-bosses and the final one.
    pub spawned: u32,
    /// One per pool entry: drawn already.
    pub used: Vec<bool>,
    /// The spawn director's, per wave (DIR1): the ticks the wave's schedule has slid, one for
    /// each tick a spawn that was due found the crowd at its target. Zero for a wave never held.
    pub held: Vec<u32>,
}

/// The one expertise gauge of the one weapon (ADR-106 §1 to §3), and its offers (ADR-079 §2, §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseGauge {
    /// From 0.
    pub level: u32,
    /// Points toward the next level; at most `next - 1` between ticks.
    pub points: u32,
    /// The points the next level needs.
    pub next: u32,
    /// The levels whose offers have opened or were passed over: those above it, up to `level`,
    /// are due, in level order.
    pub offered: u32,
    pub offer: Option<ExpertiseOffer>,
    /// For each option of the weapon's common pool, in the pool's order: times taken. Empty
    /// for a weapon with no expertise content.
    pub taken: Vec<u8>,
    /// What the range options taken add up to: every combo step's hit box that much wider,
    /// with the range rewards' (ADR-070, ADR-099).
    pub range_pct: u32,
    /// The weapon's style, a place among its three, once chosen at the level table's style
    /// level; held for the run (ADR-106 §5).
    pub style: Option<u8>,
    /// The places of the style's choices taken at the upgrade and capstone levels, in level
    /// order.
    pub path: Vec<u8>,
}

/// An open expertise offer: common options, or a style's fixed choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseOffer {
    /// The level that opened it.
    pub level: u32,
    pub kind: ExpertiseOfferKind,
    /// Places in the weapon's common pool, or among the fixed choices, in the offer's order.
    pub candidates: Vec<u8>,
}

/// Falling stars, the offers due and the open one, and what the run's offers have used
/// (ADR-079 §2, §3).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StarField {
    /// Kills in a row that dropped no star, for the pity.
    pub misses: u32,
    /// The stars waiting on the map, in the order they dropped (entity-ID order).
    pub on_map: Vec<StarState>,
    /// The offers due that have not opened yet: a boss kill's first, then the stars taken, in
    /// the order taken.
    pub due: Vec<Due>,
    pub offer: Option<OfferState>,
    pub rerolls_left: u32,
    pub banishes_left: u32,
    /// Candidates the offer-candidates rewards taken add to every offer, held to
    /// `MAX_OFFER_CANDIDATES` less the base `offer_size` of `rules.json`.
    pub extra_candidates: u8,
    /// The rarities its stars and offers may have, lowest and highest (ADR-110): fixed for the
    /// run, part of the setup, and hashed with it.
    pub rarity: (Rarity, Rarity),
    /// The runes the random rune may draw, by `Rune::index` (ADR-107): fixed for the run, part
    /// of the setup, and hashed with it.
    pub rune_pool: u8,
    /// The run's pool: places in the rewards file, in its order, fixed for the run.
    pub pool: Vec<u16>,
    /// For each reward of the file: times taken, and whether it is banished.
    pub taken: Vec<u8>,
    pub banished: Vec<bool>,
    /// For each set of the run's constellations: the members held, counted after every take
    /// (core#NS3).
    pub set_counts: Vec<u8>,
}

/// A heal pickup on the map.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct HealPickup {
    pub id: EntityId,
    pub at: Point,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StarState {
    pub id: EntityId,
    pub at: Point,
    pub rarity: Rarity,
}

/// An offer due: what opens it, and the highest rarity a candidate may have.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Due {
    pub kind: OfferKind,
    pub rarity: Rarity,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OfferState {
    pub kind: OfferKind,
    /// The highest rarity a candidate may have: the star's, or the top of the boss offer's tier.
    pub rarity: Rarity,
    /// Places in the rewards file, in the offer's order.
    pub candidates: Vec<u16>,
}

/// What a run stat does, from the run stats file.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StatRule {
    pub id: ContentId,
    pub effect: StatEffect,
    pub step: u32,
    pub limit: u8,
}

/// Experience, levels and stat points (ADR-079 §2, §4; DS18).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Progress {
    /// From 1.
    pub level: u32,
    /// Experience toward the next level.
    pub xp: u32,
    /// The run stats' rules, in the run stats' order: content, fixed for the run, not hashed.
    pub stats: Vec<StatRule>,
    /// Every stat's place in `stats`, in the order a level-up spends its point on them: the
    /// setup's stat priority, then the stats it leaves out in their own order.
    pub priority: Vec<u8>,
    /// A level-up opens a stat offer instead of spending by `priority`.
    pub pick_stats: bool,
    /// Levels reached whose stat offers have not opened yet.
    pub offers_due: u32,
    /// A stat offer is open: `Engine::choose` takes one of the stats below their limit.
    pub offer_open: bool,
    /// Points spent on each run stat, in the run stats' order.
    pub spent: Vec<u8>,
    /// What the points spent add up to for each effect, in `StatEffect::ALL` order.
    pub totals: [u32; StatEffect::ALL.len()],
}

pub(crate) use crate::id::{MAX_PLAYERS, PlayerIdx};

/// The players of a run: `1..=MAX_PLAYERS` of them. All places are held, the ones past `len`
/// as copies of the first that nothing reads, so that an index is a plain offset and a steady
/// tick allocates nothing.
#[derive(Clone, Debug)]
pub(crate) struct Players {
    items: [PlayerState; MAX_PLAYERS as usize],
    len: u8,
}

#[cfg(test)]
impl PartialEq for Players {
    fn eq(&self, other: &Players) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Players {
    pub(crate) fn solo(p: PlayerState) -> Players {
        Players {
            items: [p.clone(), p.clone(), p],
            len: 1,
        }
    }

    #[cfg(test)]
    pub(crate) fn of(ps: Vec<PlayerState>) -> Option<Players> {
        if ps.is_empty() || ps.len() > usize::from(MAX_PLAYERS) {
            return None;
        }
        let len = u8::try_from(ps.len()).ok()?;
        let first = ps[0].clone();
        let mut it = ps.into_iter();
        Some(Players {
            items: std::array::from_fn(|_| it.next().unwrap_or_else(|| first.clone())),
            len,
        })
    }

    pub(crate) fn len(&self) -> usize {
        usize::from(self.len)
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, PlayerState> {
        self.items[..self.len()].iter()
    }

    /// Players 1 and up.
    pub(crate) fn others(&self) -> &[PlayerState] {
        &self.items[1..self.len()]
    }
}

impl std::ops::Index<PlayerIdx> for Players {
    type Output = PlayerState;
    #[inline]
    fn index(&self, i: PlayerIdx) -> &PlayerState {
        debug_assert!(i.get() < self.len());
        &self.items[i.get()]
    }
}

impl std::ops::IndexMut<PlayerIdx> for Players {
    #[inline]
    fn index_mut(&mut self, i: PlayerIdx) -> &mut PlayerState {
        debug_assert!(i.get() < self.len());
        &mut self.items[i.get()]
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PlayerState {
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    /// Ticks of invulnerability left, after a hit or from an evade.
    pub invulnerable: u32,
    /// The last non-zero movement direction (ADR-069): where a combo step attacks and where an
    /// evade without movement dashes.
    pub facing: Dir,
    /// The enemy aim assist and auto mode face, picked each tick while either is on (ADR-072).
    pub target: Option<EntityId>,
    /// The evade's charges held, ready or recovering (ADR-074).
    pub evade_charges: u8,
    /// Ticks left on each spent charge, in the order they were spent; none is 0 and there are
    /// never more than `evade_charges`.
    pub evade_recovering: Vec<u32>,
    /// The buttons held on the last tick (`input::held_buttons`), so that only a button held now
    /// and not then is a press (ADR-071).
    pub held: u8,
    /// Where it is in its Main weapon's combos (ADR-070).
    pub combo: ComboState,
    /// The dash in progress, if any.
    pub dash: Option<DashState>,
    /// The stage skill slot (ADR-075): empty until a reward fills it.
    pub skill: Option<SkillSlot>,
    /// The stage skill's pull while it pulls, and its shield while it holds.
    pub pull: Option<PullState>,
    pub shield: Option<ShieldState>,
    /// The orbits rewards gave (ADR-069), in the order they came, at most `MAX_ORBITS`.
    pub orbits: Vec<OrbitState>,
    /// What the range rewards taken add up to, in percent, saturating (ADR-099).
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, saturating (ADR-099).
    pub rune_haste_pct: u32,
    /// What the pickup radius rewards taken add up to, in percent, saturating: the pickup
    /// circle of stars and heal pickups is that much wider, held to
    /// `MAX_PICKUP_RADIUS_TOTAL_PCT` where it is used.
    pub pickup_radius_pct: u32,
    /// What the skill cooldown rewards taken add up to, in percent, saturating: the stage
    /// skill's cooldown is that much shorter, held to `MAX_SKILL_COOLDOWN_TOTAL_PCT`.
    pub skill_cooldown_pct: u32,
    /// Hit points (the stage's scale) the heal-per-kills rewards taken heal together each time
    /// `kill_heal_every` kills have been counted; their amounts add up, held to
    /// `MAX_HEAL_AMOUNT` where it is used.
    pub kill_heal: u32,
    /// The kills between two such heals: the shortest interval among the picks, 0 while none
    /// is held.
    pub kill_heal_every: u32,
    /// Kills counted since the last such heal, below `kill_heal_every` between ticks.
    pub kills_counted: u32,
    /// What the crit chance rewards taken add up to, in percent, saturating; held to
    /// `MAX_CRIT_CHANCE_TOTAL_PCT` where it is rolled.
    pub crit_chance_pct: u32,
    /// What the crit damage rewards add up to: a crit does twice the damage and this many
    /// percent more, held to `MAX_CRIT_DAMAGE_TOTAL_PCT`.
    pub crit_damage_pct: u32,
    /// What the echo rewards add up to: each hit echoes this percent of its damage, held to
    /// `MAX_ECHO_HIT_PCT`; 0 while none is held.
    pub echo_hit_pct: u32,
    /// The kill burst rewards taken, added up: the percent of the dead enemy's maximum hit
    /// points, the radius in pixels, and the longest chain; all 0 while none is held.
    pub burst_pct: u32,
    pub burst_radius: u32,
    pub burst_depth: u32,
    /// Links more on the chain, from `burst_depth` rewards.
    pub burst_depth_extra: u32,
    /// What the type bonus rewards taken add up to for each damage type, by its place in
    /// `DamageType::ALL`, in percent, saturating (ADR-100): a group's bonus is added to each
    /// type of its group.
    pub type_bonus_pct: [u32; DamageType::ALL.len()],
    /// The runes the Main weapon holds, in the order drawn, each once, at most `MAX_RUNES`
    /// (ADR-099).
    pub runes: Vec<HeldRune>,
    pub aggro: u32,
    pub statuses: Statuses,
}

impl PlayerState {
    /// The evade's charges ready: held and not recovering.
    pub fn evade_ready(&self) -> u8 {
        // There are never more timers than charges, and charges are at most 3.
        self.evade_charges
            .saturating_sub(u8::try_from(self.evade_recovering.len()).expect("at most 3 charges"))
    }
}

/// Where the main character is in its Main weapon's combo tree (ADR-070, ADR-076).
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ComboState {
    /// The presses from the root to the current step, the first in bit 0: 1 for `smash`.
    pub path: u32,
    /// How many presses `path` holds: 0 at the root.
    pub depth: u8,
    /// The path began in the dash branch.
    pub dash: bool,
    /// The press waiting for the next step: `Some(true)` for `smash`.
    pub buffered: Option<bool>,
    /// The current step's attack is running.
    pub running: bool,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
    /// Ticks left in which a press from the root takes the dash branch.
    pub dash_strike_left: u32,
    /// The current step charging: ticks held, 0 on the tick it began.
    pub charge: Option<u32>,
    /// The current step held (ADR-084 point 6): ticks held, 0 on the tick it began.
    pub hold: Option<u32>,
    /// The current step's hold has released, so its release attack, and its recovery, never
    /// begins a hold again; a step taken next clears it.
    pub released: bool,
    /// A press by hand took a step in this combo (ADR-073): auto mode waits, and the combo
    /// becomes the auto combo when it resets, unless it began in the dash branch.
    pub hand: bool,
    /// The auto combo auto mode plays: its presses from the root, the first in bit 0 (1 for
    /// `smash`), and how many; at most the tree's depth.
    pub auto_path: u32,
    pub auto_depth: u8,
    /// The charge level each step of the combo was released at (ADR-114), 4 bits a step from
    /// the root, the first in the low bits (a charge has at most `MAX_CHARGE_LEVELS` = 8
    /// levels): by hand, the level the player let go at; for a step auto mode played, the level
    /// it played. 0 for a plain step.
    pub levels: u32,
    /// The same for the auto combo: the level auto mode charges each of its steps to.
    pub auto_levels: u32,
    /// The current charge is auto mode's: it needs no button and is never perfect.
    pub charge_auto: bool,
}

/// The Sub weapon's pull on what a combo part hits (ADR-070), carried by the attack and the
/// orbs it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
}

/// What a smash (a step reached by B) does to the enemy it hits (ADR-084 point 8), carried by
/// the attack and the orbs it spawns, as a combo step's hits carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Smash {
    Pull(Pull),
    /// The enemy is pushed away from the main character, as far and as long as the pull's numbers.
    Knockback(Pull),
    /// The `bleed` status, with the bleed rune's numbers from the runes file (ADR-099 point 3).
    Bleed,
    /// The `stun` status of a smash, for its ticks (ADR-085).
    Stun {
        ticks: u32,
    },
}

/// The stage skill held (ADR-075): its place in the stage skills file, its level from 1, and
/// the ticks until it is ready again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SkillSlot {
    pub index: u16,
    pub level: u8,
    pub cooldown_left: u32,
}

/// An orbit held: its reward's place in the rewards file, its orbs, the orbs of the round going
/// on, the ticks until the next round may start, and where orb 0 is round the turn (1/256
/// degree from straight down the screen, in `0..PER_TURN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbitState {
    pub reward: u16,
    pub orbs: u8,
    pub round: u8,
    pub cooldown_left: u32,
    pub angle: i32,
}

/// A rune the Main weapon holds, and for an on-hit rune the ticks until it may fire again;
/// always 0 for a damage-over-time rune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeldRune {
    pub rune: Rune,
    pub cooldown_left: u32,
}

/// A pull pulling: its point, radius and strength (fixed-point units), and the ticks left, at
/// least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PullState {
    pub at: Point,
    pub radius: i32,
    pub strength: i32,
    pub ticks_left: u32,
}

/// A shield: the damage it still absorbs, at least 1, and the ticks left, at least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ShieldState {
    pub amount: u32,
    pub ticks_left: u32,
}

/// An evade's dash: locked to its direction, `done` of its ticks moved, `left` to go.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct DashState {
    pub dir: Dir,
    pub done: u32,
    pub left: u32,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct BossState {
    pub id: EntityId,
    /// Which of the run's boss definitions (`World::bosses`) it was made from. The definition
    /// is never changed; what differs by arrival is below.
    pub boss: BossIndex,
    /// How far the boss's points (its `move_to` targets) are from the definition's: a
    /// `time_limit` boss arrives 120 pixels above the main character; zero for the stage's own.
    pub origin: Point,
    /// A `time_limit` boss at a no-call time calls no elites (core#336).
    pub no_call: bool,
    /// The boss's kind (its sprite), an index into the run's `KindTable` made at spawn.
    pub kind: KindIndex,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    pub statuses: Statuses,
    /// The phase being carried out, 0-based (ADR-036 tier 2).
    pub phase: u32,
    /// Ticks since the phase began.
    pub phase_age: u32,
    /// Ticks left in which shots do the boss and its parts no harm.
    pub invulnerable: u32,
    /// The timeline step to carry out next, and the ticks to wait before it.
    pub step: u32,
    pub wait_left: u32,
    /// A `move_to` in progress.
    pub order: Option<MoveOrder>,
    /// One per rule of the phase, in order.
    pub rules: Rules,
    /// One per part of the stage's boss, in order.
    pub parts: Vec<PartState>,
    /// Whether it has called its elites; `None` for a boss that calls none.
    pub elites_called: Option<bool>,
    /// Ticks since it arrived: 1 on its first tick. The call of its elites counts them
    /// (ADR-103), and so does the announcement of that call.
    pub age: u32,
}

/// A breakable part of the boss, with its own hit points (ADR-007).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PartState {
    pub id: EntityId,
    /// The ID the stage gave it, which `part_broken` and the `part` selector name.
    pub name: ContentId,
    /// `name` as a kind, an index into the run's `KindTable` made at spawn.
    pub kind: KindIndex,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    /// From the boss's centre, and the hurt circle's radius: both from the stage.
    pub offset: Point,
    pub radius: i32,
    pub statuses: Statuses,
}

/// Which definition an agent runs: its `Agent` in the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AgentKey {
    pub source: AgentSource,
    /// The companion index, or the wave index.
    pub index: u32,
    /// For summoned agents, the rule of the wave's enemy that summoned them.
    pub rule: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AgentSource {
    Companion,
    Wave,
    Summoned,
    /// Summoned by a rule of the boss: `index` is its phase, `rule` the rule in it.
    BossRule,
    /// Rung around a shrine: `index` is the shrine's wave.
    Guardian,
    /// Brought by a pack leader: `index` is the leader's wave.
    Escort,
}

/// A companion or an enemy: a movement and a rule list (ADR-036 tier 1).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AgentState {
    pub id: EntityId,
    /// Its definition in the run's `AgentDefs`.
    pub def: DefIndex,
    /// Its kind (its sprite), an index into the run's `KindTable` made at spawn; a hit copies it.
    pub kind: KindIndex,
    pub key: AgentKey,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    /// Ticks since it spawned.
    pub age: u32,
    /// One per rule of its agent, in order.
    pub rules: Rules,
    pub statuses: Statuses,
    /// Companions only; enemies stay at 0.
    pub aggro: u32,
    /// A `move_to` in progress, which replaces its movement.
    pub order: Option<MoveOrder>,
    /// For `orbit`, in 1/256 degree.
    pub orbit_angle: i32,
    /// Enemies only: the last push-apart found it overlapping its neighbours by more than a
    /// pixel, so it does not take its own step to its place this tick (ADR-058).
    pub crowded: bool,
    /// Enemies only: it removed itself with a `self_destruct` rule. It is at 0 hit points for
    /// the removal, but it was not defeated, so nothing credits it. It only lives until the end
    /// of the tick it is set in.
    pub burst: bool,
    /// Ticks left in which a new stun cannot land on this enemy, from the end of its last stun
    /// (ADR-085 point 4); 0 for none.
    pub stun_immune: u32,
    /// An elite's own state; `None` for every other agent.
    pub elite: Option<EliteState>,
    /// An agent that leaves the map unkilled (a goblin, or any elite whose `boss_call` is
    /// `leave`); `None` for every other agent.
    pub leave: Option<LeaveState>,
    /// A shrine's own state; `None` for every other agent. Boxed: almost no agent has one.
    pub shrine: Option<Box<ShrineState>>,
}

impl AgentState {
    /// A mimic that has not revealed yet: an object nothing touches (ADR-090).
    pub(crate) fn hidden(&self) -> bool {
        self.elite.is_some_and(|el| el.disguised)
    }

    /// An elite enemy that shows as one: `EntityView::elite` and `DomainEvent::Died::elite`. A
    /// mimic in disguise does not, until it reveals; a shrine never does, its kind (`Shrine`)
    /// already says what it is (DS44 item 1). The elite machinery (cap, aura, guardians,
    /// `ShrineActivated`) still reads `elite` itself.
    pub(crate) fn shows_elite(&self) -> bool {
        self.elite.is_some() && !self.hidden() && self.shrine.is_none()
    }

    /// The class `EntityView::kind` and `DomainEvent::Died::class` give it: `listed`, the list it
    /// is in, except that a shrine is `Shrine` (SK1). The one place that decides it.
    pub(crate) fn entity_kind(&self, listed: EntityKind) -> EntityKind {
        if self.shrine.is_some() {
            EntityKind::Shrine
        } else {
            listed
        }
    }
}

/// A shrine's fight (ADR-090).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ShrineState {
    /// Ticks since the main character touched it, 0 on the tick it did; `None` while untouched.
    pub fight: Option<u32>,
    /// The guardians that spawned, in ring order.
    pub guardians: Vec<EntityId>,
}

/// When an elite that leaves is to go (ADR-090).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct LeaveState {
    /// Ticks since it was disturbed, 0 on the tick it was; `None` while it is not.
    pub disturbed: Option<u32>,
    /// A boss came or called: it goes on this tick.
    pub go: bool,
}

/// An elite holds `home`, where it spawned, until a boss calls it (ADR-068).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct EliteState {
    pub home: Point,
    pub called: bool,
    /// What the boss's call does to it; its definition's, so not part of the hash.
    pub boss_call: BossCall,
    /// A mimic that has not revealed yet (ADR-090); false for every other elite.
    pub disguised: bool,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct MoveOrder {
    pub to: Point,
    pub ticks_left: u32,
}

/// The rule states of an agent or a boss: at most `MAX_RULES`, which the validator holds.
pub(crate) type Rules = InlineVec<RuleState, { validate::limits::MAX_RULES }>;
/// The statuses on one entity: one of each kind at most (`add_status`, `add_stun`, `add_dot` add
/// a stack to the one there), so no more than there are kinds.
pub(crate) type Statuses = InlineVec<StatusState, { StatusKind::COUNT }>;
/// The frames of an attack task: one for the pattern and one for each level of nesting, at most
/// `MAX_PATTERN_DEPTH` of them, and some to spare.
pub(crate) type Frames = InlineVec<Frame, MAX_FRAMES>;
const MAX_FRAMES: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct RuleState {
    pub cooldown_left: u32,
    /// Firings left; `UNLIMITED` if there is no limit.
    pub charges_left: u32,
    pub fired: bool,
}

/// The statuses of this version (ADR-036): the three a skill can apply, and bleed, poison and
/// burn, which only their runes apply (ADR-099). The IDs in the registry are the names of the
/// first three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// More damage dealt.
    AtkUp,
    /// More damage taken.
    Vulnerable,
    /// Slower movement.
    Slow,
    /// Damage over time, in stacks (ADR-099, ADR-084 point 8).
    Bleed,
    /// Damage over time, in stacks, of its own beside bleed's (ADR-099).
    Poison,
    /// Damage over time, in stacks, of its own beside bleed's (ADR-099).
    Burn,
    /// The enemy stops what it was doing for its ticks (ADR-085). Not a skill's status.
    Stun,
}

/// The IDs of `StatusKind::ALL`, in that order, which is the order of the discriminants
/// (`every_status_kind_is_listed`). Constants, never hashed.
static STATUS_IDS: LazyLock<[ContentId; StatusKind::COUNT]> =
    LazyLock::new(|| StatusKind::ALL.map(|k| ContentId::from(k.name())));

impl StatusKind {
    /// The statuses a skill can apply: the damage-over-time ones are not among them.
    pub const FROM_SKILLS: [StatusKind; 3] =
        [StatusKind::AtkUp, StatusKind::Vulnerable, StatusKind::Slow];

    /// The status a skill can apply whose registry ID is `name`.
    pub fn from_name(name: &str) -> Option<StatusKind> {
        StatusKind::FROM_SKILLS
            .into_iter()
            .find(|k| k.name() == name)
    }

    /// Every kind. `COUNT` sizes the lists of statuses; a kind added without a place here fails
    /// the test `every_status_kind_is_listed`.
    pub const ALL: [StatusKind; 7] = [
        StatusKind::AtkUp,
        StatusKind::Vulnerable,
        StatusKind::Slow,
        StatusKind::Bleed,
        StatusKind::Poison,
        StatusKind::Burn,
        StatusKind::Stun,
    ];
    pub const COUNT: usize = StatusKind::ALL.len();

    /// The status as a content ID, a refcount bump of the shared table: an event or a snapshot
    /// that names a status allocates nothing.
    pub fn id(self) -> ContentId {
        STATUS_IDS[self as usize].clone()
    }

    pub fn name(self) -> &'static str {
        match self {
            StatusKind::AtkUp => "atk_up",
            StatusKind::Vulnerable => "vulnerable",
            StatusKind::Slow => "slow",
            StatusKind::Bleed => "bleed",
            StatusKind::Poison => "poison",
            StatusKind::Burn => "burn",
            StatusKind::Stun => "stun",
        }
    }
}

/// One status on one entity. Applying it again refreshes the duration and adds a stack.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StatusState {
    pub kind: StatusKind,
    /// Who applied it last.
    pub source: EntityId,
    pub remaining: u32,
    /// The ticks of the application that set `remaining`, for the snapshot (ADR-103): a
    /// refresh with a longer duration sets it, a shorter one leaves both alone.
    pub total: u32,
    pub stacks: u16,
    /// Percent per stack.
    pub value_pct: u16,
    /// A damage-over-time status's own state; `None` for every other status.
    pub dot: Option<DotState>,
}

/// The filler of the unused places of a list of statuses; never read.
impl Default for StatusState {
    fn default() -> Self {
        StatusState {
            kind: StatusKind::AtkUp,
            source: EntityId(0),
            remaining: 0,
            total: 0,
            stacks: 0,
            value_pct: 0,
            dot: None,
        }
    }
}

/// What a damage-over-time status deals and when (ADR-099 points 2 and 3): its damage a stack
/// and its type, from the content that applied it; the atk-up and type bonus of its source,
/// fixed when it was last applied, as an attack's are when it starts (ADR-056); its interval,
/// and the ticks to its next tick, at least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct DotState {
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub atk_up_pct: u32,
    pub type_bonus_pct: u32,
    pub interval: u32,
    pub next_in: u32,
}

/// A direction as a vector of length about `trig::SCALE`. Angle 0 is straight down the screen
/// and positive angles turn toward the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dir {
    pub x: i32,
    pub y: i32,
}

impl Dir {
    pub const DOWN: Dir = Dir {
        x: 0,
        y: crate::trig::SCALE,
    };
    pub const UP: Dir = Dir {
        x: 0,
        y: -crate::trig::SCALE,
    };
}

/// What an attack's `aimed` directions point at, and which way it faces. This is where the
/// ways of aiming meet: enemies, companions and the boss aim at an entity; the main
/// character's combo steps aim along its facing (ADR-069). A new way of aiming (an effect's
/// target, ADR-069) is a new variant, read in `attacks::aim`, hashed in its `StableHash` impl and given its
/// heading here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aim {
    /// At the entity with this ID; straight down if it is gone.
    Entity(EntityId),
    /// Along a fixed direction.
    Fixed(Dir),
}

impl Aim {
    /// The way the attack faces when it starts: where `relative` and `sequential` directions
    /// begin. An attack at an entity faces down the screen, as the enemies do.
    pub fn heading(self) -> Dir {
        match self {
            Aim::Entity(_) => Dir::DOWN,
            Aim::Fixed(d) => d,
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OrbState {
    pub at: Point,
    pub vx: Fx,
    pub vy: Fx,
    /// The direction it was fired in, which `relative` directions of its children start from.
    pub dir: Dir,
    /// For the renderer; 0 is the default orb.
    pub style: u16,
    /// Who fired it, for the hit event and for aggro.
    pub owner: EntityId,
    pub damage: u32,
    /// The type of the phase that fired it (ADR-100); `None` is neutral.
    pub damage_type: Option<DamageType>,
    /// Its hit breaks the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub smash: Option<Smash>,
    /// Fired by a combo step, A or B: its hit adds expertise points (ADR-078 §2).
    pub combo: bool,
    /// Starts an attack of its own when it ends: an orb's burst.
    pub hook: Option<Box<Hook>>,
    /// An orb the main character's hit boxes break, with no burst.
    pub breakable: bool,
    /// Ticks left before it leaves the map (core#147). It leaves when this runs out or when it is
    /// past the map's margin, whichever comes first.
    pub life: u32,
    /// Moves left before it halts (`stop_after`, Z3a-3); `Some(0)` is halted, `None` never halts.
    pub stop: Option<u32>,
    /// The beams this orb is part of (`link`, Z3a-3).
    pub link: Option<OrbLink>,
    /// The program that fired it, which names its attack in the hit event. Not in the state hash:
    /// nothing reads it back.
    pub program: ProgramIndex,
}

/// An orb's place in a cage (Z3a-3): the orbs of one `ring`, `spread` or `repeat` round of a
/// `link` node share a `group`; slot `j` is joined to slot `(j + 1) % of` by a beam while both are
/// alive and neither is still flying toward its halt.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OrbLink {
    pub group: u32,
    /// The round of the loop that fired it.
    pub slot: u8,
    /// The slots made so far in the group, which the caps may leave short of the loop's count.
    pub of: u8,
    pub width: Fx,
    /// What a beam hit deals, already scaled by the difficulty.
    pub damage: u32,
    pub rehit_ticks: u32,
    /// The tick the beam from this orb to its neighbour last hit the main character.
    pub last_hit: Option<u32>,
}

/// A zone (core Z1): an area that does not move. It warns for `wind_left` ticks, then hurts for
/// `active_left`, then is gone. It has no ID: nothing refers to one.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ZoneState {
    /// Who placed it, for the hit event and for aggro.
    pub owner: EntityId,
    pub friendly: bool,
    pub shape: ZoneShape,
    /// The centre, fixed when it was placed.
    pub at: Point,
    /// The direction an arc or a line points, a unit vector.
    pub dir: Dir,
    /// Ticks left of the warning; 0 once it is active.
    pub wind_left: u32,
    /// Ticks left of the hurting, counted down once the warning is over.
    pub active_left: u32,
    /// The warning's length when it was placed. Snapshot only: not in the state hash.
    pub wind_ticks: u32,
    /// The hurting's length when it was placed. Snapshot only: not in the state hash.
    pub active_ticks: u32,
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// A warning only (Z3a-4): it never becomes active and never hurts.
    pub warn_only: bool,
    /// Who it has hit, and on which tick, sorted by entity ID: each once.
    pub hits: Vec<(EntityId, u32)>,
    /// The program that placed it, which names its attack in the hit event. Not in the state
    /// hash: nothing reads it back.
    pub program: ProgramIndex,
}

/// An orb's burst, waiting for the orb to end.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Hook {
    pub program: ProgramIndex,
    /// The `orb` node in the program: its `burst` list is what the orb starts.
    pub path: Vec<PathStep>,
    /// Ticks left before it starts.
    pub left: u32,
    pub owner: EntityId,
    pub aim: Aim,
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub smash: Option<Smash>,
    /// Of a combo step, A or B (ADR-078 §2).
    pub combo: bool,
}

/// One step down a program: the node at `node` in the current list, then its `body` list, or
/// its `burst` list if `then` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PathStep {
    pub node: u16,
    pub then: bool,
}

/// Where an attack's orbs and zones start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Wherever its owner is, for as long as the owner lives.
    Owner,
    /// A fixed point: where an orb was when it started this attack.
    At(Point),
}

/// A place in the middle of a list of nodes, and the state of the loop or turn whose body the
/// next frame up is running.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Frame {
    /// The node of this frame's list that is being carried out.
    pub idx: u16,
    pub loop_i: u32,
    pub loop_n: u32,
    /// A spread's angle.
    pub param: i32,
    /// The turn this node adds to the directions of its body.
    pub offset: i32,
}

/// A phased attack an attacker is carrying out (ADR-055): one at a time per attacker and slot.
/// Its phases spawn their nodes as `AttackTask`s, which run to their own end.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackRun {
    pub owner: EntityId,
    /// 0 for an agent's or the boss's attacks and the main character's combo (ADR-069).
    pub slot: u8,
    /// The attack it carries out, compiled when the run started (`World::attacks`).
    pub attack: AttackIndex,
    /// The attack speed that shortens its phases, in percent, and the range that widens its hit
    /// boxes, fixed when it starts (ADR-070); read through `attacks::phase`.
    pub speed_pct: u32,
    pub width_pct: u32,
    pub phase: u32,
    /// Ticks done in the phase: 0 on its first tick.
    pub tick: u32,
    /// Locked when the attack starts.
    pub facing: Dir,
    pub aim: Aim,
    pub friendly: bool,
    /// Started by a `ranged` rule: while in wind-up or active it holds a ranged token (core Z2).
    pub ranged: bool,
    /// The damage its starter gives a phase that sets none (ADR-055): the main character's
    /// attack, a shot skill's own damage, 1 for an enemy or the boss.
    pub base: u32,
    /// The attacker's atk-up, in percent, fixed when the attack starts (ADR-056).
    pub atk_up_pct: u32,
    /// The percent of the damage a difficulty scales an enemy's or the boss's attack by
    /// (ADR-110), fixed when the attack starts; 100 for the main character's and its allies'.
    pub damage_pct: u32,
    /// The attacker's type bonus for each damage type, by its place in `DamageType::ALL`, in
    /// percent, each at most the match-up file's `max_type_bonus_pct`, fixed when the attack
    /// starts (ADR-100): the main character's, once it holds any; none for every other
    /// attacker. Boxed, so that a run without one stays as small as it was.
    pub type_bonus_pct: Option<Box<[u32; DamageType::ALL.len()]>>,
    /// `phase_time` of the attack's first tick, for the nodes its phases spawn.
    pub age: u32,
    /// Who the current active phase has hit, and on which tick last, sorted by entity ID;
    /// emptied when an active phase begins.
    pub hits: Vec<(EntityId, u32)>,
    /// The Sub weapon's pull, when the attack is a combo part (ADR-070).
    pub smash: Option<Smash>,
    /// Where its hit boxes are centred instead of on its attacker: an on-hit rune's attack, at
    /// the point the combo step hit (ADR-099), or where a thrown one has flown to.
    pub at: Option<Point>,
    /// A thrown rune's turn point, the point the combo step hit: its first phase flies `at`
    /// there from the main character, its second back to the main character (boomerang). A
    /// homing missile's goal: where its target is, or was last.
    pub throw: Option<Point>,
    /// A homing missile's target (ADR-099): `throw` follows it while it is alive, and the
    /// missile is gone once it hits anything.
    pub homing: Option<EntityId>,
    /// Where its target stood when it began, for the hit boxes and zones anchored at the target
    /// (core Z1); only an attack that has one holds it.
    pub spot: Option<Point>,
}

/// The movement speed, in percent, that the running attacks leave each attacker: the lowest
/// `move_pct` of the current phases of its runs. Kept beside `State::runs` so that asking costs
/// a search over the attackers, not a scan of every run; a derived value that no hash reads.
/// Sorted by owner, one entry for each owner with a run.
#[derive(Clone, Debug, Default)]
pub(crate) struct MoveIndex {
    by_owner: Vec<(EntityId, i64)>,
}

impl MoveIndex {
    /// Room for `owners` entries (DIR1): one per attacker that can have a run.
    pub fn reserve(&mut self, owners: usize) {
        crate::sim::scratch::reserve_to(&mut self.by_owner, owners);
    }

    /// The speed `owner`'s runs leave it, 100 with none.
    #[inline]
    pub fn get(&self, owner: EntityId) -> i64 {
        let at = self.by_owner.partition_point(|e| e.0 < owner);
        match self.by_owner.get(at) {
            Some(e) if e.0 == owner => e.1,
            _ => 100,
        }
    }

    /// Takes in a run that starts, or whose phase changes to one, with this `move_pct`.
    pub fn lower(&mut self, owner: EntityId, pct: i64) {
        let at = self.by_owner.partition_point(|e| e.0 < owner);
        match self.by_owner.get_mut(at) {
            Some(e) if e.0 == owner => e.1 = e.1.min(pct),
            _ => self.by_owner.insert(at, (owner, pct)),
        }
    }

    /// Starts again from `runs`: after one ended or went on to another phase.
    pub fn rebuild(&mut self, world: &World, runs: &[AttackRun]) {
        self.by_owner.clear();
        self.by_owner.extend(runs.iter().map(|r| {
            let phase = &world.attacks[usize::from(r.attack.0)].phases[r.phase as usize];
            (r.owner, i64::from(phase.move_pct))
        }));
        // Lowest first within an owner, so that the entry `dedup_by_key` keeps is the lowest.
        self.by_owner.sort_unstable();
        self.by_owner.dedup_by_key(|e| e.0);
    }
}

/// An attack being carried out (ADR-036 tier 3): a program, where it has got to, and what it
/// needs to know to aim and to hurt. Presets are expanded before they get here, so a preset and
/// its inline copy are the same task.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackTask {
    pub owner: EntityId,
    pub program: ProgramIndex,
    /// Where in the program this task starts; empty for the whole program.
    pub base: Vec<PathStep>,
    pub origin: Origin,
    /// What `aimed` directions point at.
    pub aim: Aim,
    pub friendly: bool,
    pub damage: u32,
    /// The type of the phase that started it, which its orbs carry (ADR-100).
    pub damage_type: Option<DamageType>,
    /// The orbs it fires break the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub smash: Option<Smash>,
    /// Started by a combo step, A or B: its hits add expertise points (ADR-078 §2).
    pub combo: bool,
    /// Ticks since it started.
    pub age: u32,
    /// Ticks to wait before going on.
    pub wait_left: u32,
    /// The stack of lists being carried out, outermost first.
    pub frames: Frames,
    /// The direction `relative` directions start from.
    pub parent_dir: Dir,
    /// The direction of the last orb, which `sequential` directions turn from.
    pub last_dir: Dir,
    /// The spot of the attack that started it (`AttackRun::spot`), for its zones anchored at the
    /// target.
    pub spot: Option<Point>,
}

#[cfg(test)]
mod status_kind_tests {
    use super::StatusKind;

    #[test]
    fn every_status_kind_is_listed() {
        // An exhaustive match: a kind added without a number here does not compile, and one
        // without a place in `ALL` fails the assert below, so `Statuses` is sized for all.
        let number = |k: StatusKind| match k {
            StatusKind::AtkUp => 0,
            StatusKind::Vulnerable => 1,
            StatusKind::Slow => 2,
            StatusKind::Bleed => 3,
            StatusKind::Poison => 4,
            StatusKind::Burn => 5,
            StatusKind::Stun => 6,
        };
        let numbers: Vec<usize> = StatusKind::ALL.into_iter().map(number).collect();
        assert_eq!(numbers, (0..StatusKind::COUNT).collect::<Vec<_>>());
    }

    #[test]
    fn a_status_kind_is_its_own_id() {
        for (i, kind) in StatusKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i);
            assert_eq!(kind.id().as_str(), kind.name());
        }
    }
}
