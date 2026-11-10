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
use crate::rng::{
    SplitMix64, TAG_ATTACK, TAG_HORDE, TAG_LOOT, TAG_OFFER, TAG_OFFER_1, TAG_OFFER_2, TAG_SPAWN,
    derive,
};
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::runes::Runes;
use crate::sim::equipment::Equipment;
use crate::sim::{AttackIndex, BossIndex, DefIndex, ProgramIndex, RangedPool, World};
use crate::snapshot::{EntityKind, OfferKind};
use crate::validate::{self};
use std::sync::LazyLock;

/// Firings or casts left when there is no limit.
pub(crate) const UNLIMITED: u32 = u32::MAX;

/// The ticks a player stays downed before becoming out for the run (ADR-SIM-7).
pub(crate) const OUT_TIMER_TICKS: u32 = 1200;
/// The ticks a teammate in range must hold interact to revive (ADR-SIM-7).
pub(crate) const REVIVE_HOLD_TICKS: u32 = 180;
/// Hit points restored on revive, in percent of max HP (ADR-SIM-7).
pub(crate) const REVIVE_HP_PCT: u32 = 30;
/// The interaction radius within which a teammate can revive (ADR-SIM-7).
pub(crate) const REVIVE_RANGE: Fx = Fx(12_288);

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
#[derive(Clone, Debug)]
pub(crate) struct State {
    /// Buffers reused from tick to tick; not part of the state (not hashed, not cloned).
    pub scratch: crate::sim::Scratch,
    /// The run's kinds, fixed at its start (core#271).
    pub kinds: KindTable,
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
    pub runes: Runes,
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
#[derive(Debug)]
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

impl Clone for Horde {
    fn clone(&self) -> Horde {
        let Horde {
            boss_ticks,
            no_call_ticks,
            mid_boss_hp_pct,
            spawned,
            used,
            held,
        } = self;
        Horde {
            boss_ticks: boss_ticks.clone(),
            no_call_ticks: no_call_ticks.clone(),
            mid_boss_hp_pct: mid_boss_hp_pct.clone(),
            spawned: *spawned,
            used: used.clone(),
            held: held.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Horde) {
        let Horde {
            boss_ticks,
            no_call_ticks,
            mid_boss_hp_pct,
            spawned,
            used,
            held,
        } = self;
        let Horde {
            boss_ticks: s_boss_ticks,
            no_call_ticks: s_no_call_ticks,
            mid_boss_hp_pct: s_mid_boss_hp_pct,
            spawned: s_spawned,
            used: s_used,
            held: s_held,
        } = source;
        boss_ticks.clone_from(s_boss_ticks);
        no_call_ticks.clone_from(s_no_call_ticks);
        mid_boss_hp_pct.clone_from(s_mid_boss_hp_pct);
        *spawned = *s_spawned;
        used.clone_from(s_used);
        held.clone_from(s_held);
    }
}

/// The one expertise gauge of the one weapon, and its offers (ADR-REPL-5).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseGauge {
    /// From 0.
    pub level: u32,
    /// Points toward the next level; at most `next - 1` between ticks.
    pub points: u32,
    /// The points the next level needs.
    pub next: u32,
    /// What the range options taken add up to: every combo step's hit box that much wider,
    /// with the range rewards'.
    pub range_pct: u32,
    /// The weapon's style, a place among its three, once chosen at the level table's style
    /// level; held for the run.
    pub style: Option<u8>,
    /// The places of the style's choices taken at the upgrade and capstone levels, in level
    /// order.
    pub path: Vec<u8>,
}

impl Clone for ExpertiseGauge {
    fn clone(&self) -> ExpertiseGauge {
        let ExpertiseGauge {
            level,
            points,
            next,
            range_pct,
            style,
            path,
        } = self;
        ExpertiseGauge {
            level: *level,
            points: *points,
            next: *next,
            range_pct: *range_pct,
            style: *style,
            path: path.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ExpertiseGauge) {
        let ExpertiseGauge {
            level,
            points,
            next,
            range_pct,
            style,
            path,
        } = self;
        let ExpertiseGauge {
            level: s_level,
            points: s_points,
            next: s_next,
            range_pct: s_range_pct,
            style: s_style,
            path: s_path,
        } = source;
        *level = *s_level;
        *points = *s_points;
        *next = *s_next;
        *range_pct = *s_range_pct;
        *style = *s_style;
        path.clone_from(s_path);
    }
}

/// The run part of the star field: falling stars and what the run's setup fixes for every
/// player's offers (ADR-REPL-5). The offers due and open, and what each player's offers have
/// used, are per slot in `Offers` (ADR-SIM-7).
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StarField {
    /// Kills in a row that dropped no star, for the pity.
    pub misses: u32,
    /// The stars waiting on the map, in the order they dropped (entity-ID order).
    pub on_map: Vec<StarState>,
    /// The rarities its stars and offers may have, lowest and highest: fixed for the
    /// run, part of the setup, and hashed with it.
    pub rarity: (Rarity, Rarity),
    /// The runes the random rune may draw, by `Rune::index`: fixed for the run, part
    /// of the setup, and hashed with it.
    pub rune_pool: u8,
    /// The run's pool: places in the rewards file, in its order, fixed for the run.
    pub pool: Vec<u16>,
}

impl Clone for StarField {
    fn clone(&self) -> StarField {
        let StarField {
            misses,
            on_map,
            rarity,
            rune_pool,
            pool,
        } = self;
        StarField {
            misses: *misses,
            on_map: on_map.clone(),
            rarity: *rarity,
            rune_pool: *rune_pool,
            pool: pool.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &StarField) {
        let StarField {
            misses,
            on_map,
            rarity,
            rune_pool,
            pool,
        } = self;
        let StarField {
            misses: s_misses,
            on_map: s_on_map,
            rarity: s_rarity,
            rune_pool: s_rune_pool,
            pool: s_pool,
        } = source;
        *misses = *s_misses;
        on_map.clone_from(s_on_map);
        *rarity = *s_rarity;
        *rune_pool = *s_rune_pool;
        pool.clone_from(s_pool);
    }
}

/// One player's star offers: those due and the open one, and what the player's offers have used
/// (ADR-REPL-5). A slot holds `Some` exactly when the stage has stars (`State::stars`).
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Offers {
    /// The offers due that have not opened yet: a boss kill's first, then the stars taken, in
    /// the order taken.
    pub due: Vec<Due>,
    pub offer: Option<OfferState>,
    pub rerolls_left: u32,
    pub banishes_left: u32,
    /// Candidates the offer-candidates rewards taken add to every offer, held to
    /// `MAX_OFFER_CANDIDATES` less the base `offer_size` of `rules.json`.
    pub extra_candidates: u8,
    /// For each reward of the file: times taken, and whether it is banished.
    pub taken: Vec<u8>,
    pub banished: Vec<bool>,
    /// For each set of the run's constellations: the members held, counted after every take
    /// (core#NS3).
    pub set_counts: Vec<u8>,
}

impl Clone for Offers {
    fn clone(&self) -> Offers {
        let Offers {
            due,
            offer,
            rerolls_left,
            banishes_left,
            extra_candidates,
            taken,
            banished,
            set_counts,
        } = self;
        Offers {
            due: due.clone(),
            offer: offer.clone(),
            rerolls_left: *rerolls_left,
            banishes_left: *banishes_left,
            extra_candidates: *extra_candidates,
            taken: taken.clone(),
            banished: banished.clone(),
            set_counts: set_counts.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Offers) {
        let Offers {
            due,
            offer,
            rerolls_left,
            banishes_left,
            extra_candidates,
            taken,
            banished,
            set_counts,
        } = self;
        let Offers {
            due: s_due,
            offer: s_offer,
            rerolls_left: s_rerolls_left,
            banishes_left: s_banishes_left,
            extra_candidates: s_extra_candidates,
            taken: s_taken,
            banished: s_banished,
            set_counts: s_set_counts,
        } = source;
        due.clone_from(s_due);
        offer.clone_from(s_offer);
        *rerolls_left = *s_rerolls_left;
        *banishes_left = *s_banishes_left;
        *extra_candidates = *s_extra_candidates;
        taken.clone_from(s_taken);
        banished.clone_from(s_banished);
        set_counts.clone_from(s_set_counts);
    }
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

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OfferState {
    pub kind: OfferKind,
    /// The highest rarity a candidate may have: the star's, or the top of the boss offer's tier.
    pub rarity: Rarity,
    /// Places in the rewards file, in the offer's order.
    pub candidates: Vec<u16>,
}

impl Clone for OfferState {
    fn clone(&self) -> OfferState {
        let OfferState {
            kind,
            rarity,
            candidates,
        } = self;
        OfferState {
            kind: *kind,
            rarity: *rarity,
            candidates: candidates.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &OfferState) {
        let OfferState {
            kind,
            rarity,
            candidates,
        } = self;
        let OfferState {
            kind: s_kind,
            rarity: s_rarity,
            candidates: s_candidates,
        } = source;
        *kind = *s_kind;
        *rarity = *s_rarity;
        candidates.clone_from(s_candidates);
    }
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

/// Experience, levels and stat points (ADR-REPL-5; DS18).
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
    /// Points spent on each run stat, in the run stats' order.
    pub spent: Vec<u8>,
    /// What the points spent add up to for each effect, in `StatEffect::ALL` order.
    pub totals: [u32; StatEffect::ALL.len()],
    /// What the weapon's stat set adds for each effect, in tenths of a percent (`MaxHp`'s set is
    /// whole and lives in `totals`), in `StatEffect::ALL` order. Only the set's writer
    /// (`expertise.rs`) touches it; a read adds `value / 10` to the whole percent in `totals`.
    pub set_tenths: [u32; StatEffect::ALL.len()],
}

pub(crate) use crate::id::{MAX_PLAYERS, PlayerIdx};

/// What one player carries through the run besides the player's body: the experience and levels
/// (`None` in a stage without levels), the offer generator and the star offers (`None` in a stage
/// without stars), the expertise gauge, the Stardust earned and the regeneration. One per slot in
/// `Players` (ADR-SIM-7, ADR-SIM-2).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Career {
    pub progress: Option<Progress>,
    /// The star, expertise and rune offers' generator: `derive(seed, tag)` with the slot's tag
    /// (`TAG_OFFER` for slot 0).
    pub offer: SplitMix64,
    pub offers: Option<Offers>,
    /// The expertise gauge; `None` in a stage whose enemies and boss give no expertise points.
    pub expertise: Option<ExpertiseGauge>,
    /// The Stardust the run has earned, times 10: `Some` in a stage with a Stardust
    /// factor.
    pub stardust: Option<u32>,
    /// Hit points the main character regenerates each second (a perk): set at the start
    /// and raised by rewards.
    pub regen: u32,
    /// The star equipment: empty in an offers run (ADR-SIM-3 point 2).
    pub equipment: Equipment,
}

/// The offer generator's tag of slot `me`.
fn offer_tag(me: PlayerIdx) -> u64 {
    match me.get() {
        0 => TAG_OFFER,
        1 => TAG_OFFER_1,
        2 => TAG_OFFER_2,
        _ => unreachable!("{MAX_PLAYERS} slots"),
    }
}

impl Career {
    /// The career of slot `me` at the start of a run seeded `seed`.
    pub(crate) fn new(
        seed: u64,
        me: PlayerIdx,
        progress: Option<Progress>,
        offers: Option<Offers>,
        expertise: Option<ExpertiseGauge>,
        stardust: Option<u32>,
        equipment: Equipment,
    ) -> Career {
        Career {
            progress,
            offer: derive(seed, offer_tag(me)),
            offers,
            expertise,
            stardust,
            regen: 0,
            equipment,
        }
    }

    /// The career of slot `me`, a member who starts or joins, from slot 0's start career `c0`: its
    /// progress, offers, expertise, Stardust and regeneration as they were at the start (the perk's
    /// and a regen Lumine star's), the equipment of `for_new_slot`, its own offer stream.
    pub(crate) fn member(seed: u64, me: PlayerIdx, c0: &Career) -> Career {
        Career {
            regen: c0.regen,
            ..Career::new(
                seed,
                me,
                c0.progress.clone(),
                c0.offers.clone(),
                c0.expertise.clone(),
                c0.stardust,
                c0.equipment.for_new_slot(),
            )
        }
    }
}

/// An absent slot's placeholder: nothing reads or hashes it.
impl Default for Career {
    fn default() -> Career {
        Career {
            progress: None,
            offer: SplitMix64::new(0),
            offers: None,
            expertise: None,
            stardust: None,
            regen: 0,
            equipment: Equipment::default(),
        }
    }
}

/// The players of a run: `1..=MAX_PLAYERS` of them, in the slots a presence mask names. All
/// places are held, the absent ones as copies of the first that nothing reads, so that an index
/// is a plain offset and a steady tick allocates nothing. Slot 0 is always present. The entity
/// numbers 0 to 2 are the slots whether or not one is present (ADR-SIM-7).
///
/// Each slot also holds its `Career` (the level progress, the offer generator, the star offers,
/// the expertise gauge, the Stardust and the regeneration). The state hash and the savepoint write
/// the shared fields, then one presence byte (bit `i` is slot `i`) and, for each present slot in
/// ascending order, its player and its career (ADR-SIM-3, ADR-SAVE-4); an absent slot writes
/// nothing, and a hole in the mask hashes and saves. A party builder supplies a career for every
/// present slot.
#[derive(Clone, Debug)]
pub(crate) struct Players {
    items: [PlayerState; MAX_PLAYERS as usize],
    /// The career of each slot; an absent slot holds `Career::default()`, which nothing reads.
    careers: [Career; MAX_PLAYERS as usize],
    /// Bit `i` is set when slot `i` is in the party.
    present: u8,
    /// The generation of each slot's current or last occupant: `leave` adds one, so the next
    /// occupant's entity number matches nothing the leaver owned (ADR-SIM-7). Hashed and saved
    /// for every slot, present or not.
    generations: [u8; MAX_PLAYERS as usize],
}

#[cfg(test)]
impl PartialEq for Players {
    fn eq(&self, other: &Players) -> bool {
        self.present == other.present
            && self.generations == other.generations
            && self.iter().eq(other.iter())
            && self.indices().all(|i| self.career(i) == other.career(i))
    }
}

impl Players {
    /// The party of `n` slots (1 to `MAX_PLAYERS`) that grows from `old`'s first player: every
    /// slot a copy of that player, slot 0 keeping its career and each other slot a career of its
    /// own (its offer stream by `offer_tag`) from slot 0's progress, offers, expertise and
    /// Stardust. Slots 1 and 2 take the run's loadout until the per-player setup.
    pub(crate) fn party(n: u8, old: &Players, crit_seed: u64) -> Players {
        assert!((1..=MAX_PLAYERS).contains(&n), "1 to {MAX_PLAYERS} players");
        let first = old.items[0].clone();
        let c0 = old.careers[0].clone();
        let mut out = Players {
            items: [first.clone(), first.clone(), first],
            careers: Default::default(),
            present: (1u8 << n) - 1,
            generations: [0; MAX_PLAYERS as usize],
        };
        out.careers[0] = c0.clone();
        for i in 1..n {
            let me = PlayerIdx::new(i).expect("a slot");
            out.careers[me.get()] = Career::member(crit_seed, me, &c0);
        }
        out
    }

    /// The mask of a party of one.
    const SOLO_MASK: u8 = 0b001;

    /// The party of one that holds slot 0's player and career as they are now: the joiner
    /// template an engine keeps from its start.
    pub(crate) fn template(&self) -> Players {
        Players::solo(self.items[0].clone(), self.careers[0].clone())
    }

    /// Slot `me` joins: a player built as `party` builds slots 1 and 2 from `template`'s first
    /// slot (the run's starting player at level 1, its own offer stream by `offer_tag`), placed
    /// where slot 0 is now and facing as it faces. `me` is a free slot other than 0.
    pub(crate) fn join(&mut self, me: PlayerIdx, template: &Players, crit_seed: u64) {
        assert!(me != PlayerIdx::SOLO && !self.is_present(me), "a free slot");
        let c0 = &template.careers[0];
        let mut player = template.items[0].clone();
        player.at = self.items[0].at;
        player.facing = self.items[0].facing;
        self.items[me.get()] = player;
        self.careers[me.get()] = Career::member(crit_seed, me, c0);
        self.present |= 1 << me.get();
    }

    /// Slot `me` leaves: its bit is cleared and its place is filled as `from_parts` fills an
    /// absent one, which nothing reads or hashes. Slot 0 never leaves.
    pub(crate) fn leave(&mut self, me: PlayerIdx) {
        assert!(
            me != PlayerIdx::SOLO && self.is_present(me),
            "a present slot"
        );
        self.items[me.get()] = self.items[0].clone();
        self.careers[me.get()] = Career::default();
        self.generations[me.get()] = self.generations[me.get()]
            .checked_add(1)
            .expect("a slot below generation 255");
        self.present &= !(1 << me.get());
    }

    pub(crate) fn solo(p: PlayerState, career: Career) -> Players {
        Players {
            items: [p.clone(), p.clone(), p],
            careers: [career, Career::default(), Career::default()],
            present: Self::SOLO_MASK,
            generations: [0; MAX_PLAYERS as usize],
        }
    }

    /// The party `mask` from the players and careers of its present slots, in slot order (the
    /// savepoint's decode). An absent slot is filled as `solo` fills it. `mask` has slot 0 and
    /// `items` and `careers` have one entry per set bit.
    pub(crate) fn from_parts(
        mask: u8,
        generations: [u8; MAX_PLAYERS as usize],
        items: Vec<PlayerState>,
        careers: Vec<Career>,
    ) -> Players {
        assert!(mask & Self::SOLO_MASK != 0 && mask < 1 << MAX_PLAYERS);
        assert!(items.len() == mask.count_ones() as usize && items.len() == careers.len());
        let first = items[0].clone();
        let (mut items, mut careers) = (items.into_iter(), careers.into_iter());
        let mut out = Players {
            items: [first.clone(), first.clone(), first],
            careers: Default::default(),
            present: mask,
            generations,
        };
        for slot in 0..MAX_PLAYERS as usize {
            if mask >> slot & 1 == 1 {
                out.items[slot] = items.next().expect("one player per set bit");
                out.careers[slot] = careers.next().expect("one career per set bit");
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn of(ps: Vec<PlayerState>) -> Option<Players> {
        if ps.is_empty() || ps.len() > usize::from(MAX_PLAYERS) {
            return None;
        }
        let present = (1u8 << ps.len()) - 1;
        let first = ps[0].clone();
        let mut it = ps.into_iter();
        Some(Players {
            items: std::array::from_fn(|_| it.next().unwrap_or_else(|| first.clone())),
            careers: Default::default(),
            present,
            generations: [0; MAX_PLAYERS as usize],
        })
    }

    /// A party of the slots in `mask`, each a copy of `p` (tests).
    #[cfg(test)]
    pub(crate) fn with_mask(mask: u8, p: PlayerState) -> Players {
        assert!(mask & Self::SOLO_MASK != 0, "slot 0 is always present");
        assert!(mask < 1 << MAX_PLAYERS, "a mask of {MAX_PLAYERS} slots");
        Players {
            items: [p.clone(), p.clone(), p],
            careers: Default::default(),
            present: mask,
            generations: [0; MAX_PLAYERS as usize],
        }
    }

    /// This party with slot 0's career taken from `old`: a test that swaps the players of a live
    /// state keeps its progress, offer generator and offers.
    #[cfg(test)]
    pub(crate) fn keeping_career_of(mut self, old: &Players) -> Players {
        self.careers[0] = old.career(PlayerIdx::SOLO).clone();
        // The offers go with the star field in every present slot (the hash and the savepoint
        // check it): the others start with slot 0's offers, and nothing else of its career.
        for me in self.indices().skip(1) {
            self.careers[me.get()].offers = self.careers[0].offers.clone();
        }
        self
    }

    /// The slots in the party, bit `i` for slot `i`.
    #[inline]
    pub(crate) fn mask(&self) -> u8 {
        self.present
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.present.count_ones() as usize
    }

    #[inline]
    pub(crate) fn is_present(&self, i: PlayerIdx) -> bool {
        self.present >> i.get() & 1 == 1
    }

    /// The players of the party, in slot order.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PlayerState> + '_ {
        self.items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.present >> i & 1 == 1)
            .map(|(_, p)| p)
    }

    /// The `PlayerIdx` of each player of the party, in slot order. It holds a copy of the mask,
    /// not the party, so a loop over it may borrow the state mutably.
    pub(crate) fn indices(&self) -> impl Iterator<Item = PlayerIdx> + use<> {
        // Walks the set bits lowest first: slot order, no scan of the absent slots. `present`
        // never holds a bit at or past `MAX_PLAYERS` (the savepoint refuses such a mask).
        let mut rest = self.present;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// The `PlayerIdx` of each party player after slot 0, in slot order: nothing in a party of one.
    /// Slot 0 is always in the party (the savepoint refuses a mask without it), so
    /// `once(SOLO).chain(after_first())` is `indices()` without a test of slot 0.
    #[inline]
    pub(crate) fn after_first(&self) -> impl Iterator<Item = PlayerIdx> + use<> {
        let mut rest = self.present & !1;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// The entity number of slot `me` (ADR-SIM-7): the number its shots, hits and dots carry.
    #[inline]
    pub(crate) fn id(&self, me: PlayerIdx) -> EntityId {
        me.id_at(self.generations[me.get()])
    }

    /// The generation of slot `me`'s current or last occupant.
    #[inline]
    pub(crate) fn generation(&self, me: PlayerIdx) -> u8 {
        self.generations[me.get()]
    }

    /// Sets a slot's generation (tests).
    #[cfg(test)]
    pub(crate) fn set_generation_for_test(&mut self, me: PlayerIdx, generation: u8) {
        self.generations[me.get()] = generation;
    }

    /// The slots that are present and up (alive, not out), bit `i` for slot `i` (ADR-SIM-7).
    #[inline]
    pub(crate) fn up_mask(&self) -> u8 {
        let mut rest = self.present;
        let mut up = 0;
        while rest != 0 {
            let i = rest.trailing_zeros() as usize;
            rest &= rest - 1;
            up |= u8::from(self.items[i].is_up()) << i;
        }
        up
    }

    /// Only slot 0 is present: nearly every run, and the path every per-player step keeps cheap.
    #[inline]
    pub(crate) fn is_solo(&self) -> bool {
        self.present == Self::SOLO_MASK
    }

    /// The slots the world's targeting and bounds read: the up players, or every present slot
    /// when none is up (the tick a party wipes; the run fails at that tick's outcome).
    #[inline]
    pub(crate) fn targets(&self) -> u8 {
        // A party of one, nearly every run: slot 0 is the target up or down.
        if self.present == Self::SOLO_MASK {
            return Self::SOLO_MASK;
        }
        match self.up_mask() {
            0 => self.present,
            up => up,
        }
    }

    /// The slots set in `mask` (a mask of this type's slots), lowest first.
    #[inline]
    pub(crate) fn slots(mask: u8) -> impl Iterator<Item = PlayerIdx> + use<> {
        let mut rest = mask;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// Whether any player in the party is alive and not out (ADR-SIM-7).
    #[inline]
    pub(crate) fn any_up(&self) -> bool {
        self.up_count() > 0
    }

    /// How many players in the party are alive and not out (ADR-SIM-7).
    #[inline]
    pub(crate) fn up_count(&self) -> usize {
        self.iter().filter(|p| p.is_up()).count()
    }

    /// The generation of every slot, in slot order (the hash and the savepoint write them).
    #[inline]
    pub(crate) fn generations(&self) -> [u8; MAX_PLAYERS as usize] {
        self.generations
    }

    /// The present slot whose entity number is `id` (ADR-SIM-7): the owner of a shot, a hit or a
    /// dot a player made. `None` for a companion, an enemy or a slot not in the party.
    #[inline]
    pub(crate) fn of_id(&self, id: EntityId) -> Option<PlayerIdx> {
        let (me, generation) = crate::id::player_of(id)?;
        (self.is_present(me) && self.generations[me.get()] == generation).then_some(me)
    }

    /// Players 1 and up that are present, in slot order.
    #[cfg(test)]
    pub(crate) fn others(&self) -> impl Iterator<Item = &PlayerState> + '_ {
        self.iter().skip(1)
    }

    /// The career of player `me`.
    #[inline]
    pub(crate) fn career(&self, me: PlayerIdx) -> &Career {
        debug_assert!(self.is_present(me));
        &self.careers[me.get()]
    }

    #[inline]
    pub(crate) fn career_mut(&mut self, me: PlayerIdx) -> &mut Career {
        debug_assert!(self.is_present(me));
        &mut self.careers[me.get()]
    }

    /// Player `me` and the career of `me` together: a split borrow of the two lists.
    #[inline]
    pub(crate) fn both_mut(&mut self, me: PlayerIdx) -> (&mut PlayerState, &mut Career) {
        debug_assert!(self.is_present(me));
        (&mut self.items[me.get()], &mut self.careers[me.get()])
    }

    /// The careers of players 1 and up that are present, in slot order, like `others`.
    #[cfg(test)]
    pub(crate) fn other_careers(&self) -> impl Iterator<Item = &Career> + '_ {
        self.careers
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, _)| self.present >> i & 1 == 1)
            .map(|(_, c)| c)
    }

    /// All three careers, absent slots included (tests).
    #[cfg(test)]
    pub(crate) fn careers_mut_for_test(&mut self) -> &mut [Career; MAX_PLAYERS as usize] {
        &mut self.careers
    }
}

impl std::ops::Index<PlayerIdx> for Players {
    type Output = PlayerState;
    #[inline]
    fn index(&self, i: PlayerIdx) -> &PlayerState {
        debug_assert!(self.is_present(i));
        &self.items[i.get()]
    }
}

impl std::ops::IndexMut<PlayerIdx> for Players {
    #[inline]
    fn index_mut(&mut self, i: PlayerIdx) -> &mut PlayerState {
        debug_assert!(self.is_present(i));
        &mut self.items[i.get()]
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PlayerState {
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears.
    pub defence: u32,
    /// Ticks of invulnerability left, after a hit or from an evade.
    pub invulnerable: u32,
    /// The last non-zero movement direction: where a combo step attacks and where an
    /// evade without movement dashes.
    pub facing: Dir,
    /// The enemy aim assist and auto mode face, picked each tick while either is on.
    pub target: Option<EntityId>,
    /// The evade's charges held, ready or recovering.
    pub evade_charges: u8,
    /// Ticks left on each spent charge, in the order they were spent; none is 0 and there are
    /// never more than `evade_charges`.
    pub evade_recovering: Vec<u32>,
    /// The buttons held on the last tick (`input::held_buttons`), so that only a button held now
    /// and not then is a press.
    pub held: u8,
    /// Where it is in its Main weapon's combos.
    pub combo: ComboState,
    /// The dash in progress, if any.
    pub dash: Option<DashState>,
    /// The stage skill slot: empty until a reward fills it.
    pub skill: Option<SkillSlot>,
    /// The stage skill's pull while it pulls, and its shield while it holds.
    pub pull: Option<PullState>,
    pub shield: Option<ShieldState>,
    /// The orbits rewards gave, in the order they came, at most `MAX_ORBITS`.
    pub orbits: Vec<OrbitState>,
    /// What the range rewards taken add up to, in percent, saturating.
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, saturating.
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
    /// `DamageType::ALL`, in percent, saturating: a group's bonus is added to each
    /// type of its group.
    pub type_bonus_pct: [u32; DamageType::ALL.len()],
    /// The runes the Main weapon holds, in the order drawn, each once, at most `MAX_RUNES`
    ///.
    pub runes: Vec<HeldRune>,
    pub aggro: u32,
    pub statuses: Statuses,
    /// Ticks elapsed since downed (0 while up or out, ADR-SIM-7).
    pub down_ticks: u32,
    /// Ticks accumulated toward revive while interact is held in range (0 while up or out, ADR-SIM-7).
    pub revive_hold: u32,
    /// Whether the player is permanently out for the run (ADR-SIM-7).
    pub out: bool,
}

impl PlayerState {
    /// Alive and not out (ADR-SIM-7).
    #[inline]
    pub fn is_up(&self) -> bool {
        self.hp > 0 && !self.is_out()
    }

    /// At 0 HP and not yet permanently out (ADR-SIM-7).
    #[inline]
    pub fn is_downed(&self) -> bool {
        self.hp == 0 && !self.is_out()
    }

    /// Permanently out for the run (ADR-SIM-7).
    #[inline]
    pub fn is_out(&self) -> bool {
        self.out
    }

    /// The evade's charges ready: held and not recovering.
    pub fn evade_ready(&self) -> u8 {
        // There are never more timers than charges, and charges are at most 3.
        self.evade_charges
            .saturating_sub(u8::try_from(self.evade_recovering.len()).expect("at most 3 charges"))
    }
}

/// Where the main character is in its Main weapon's combo tree.
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
    /// The current step held: ticks held, 0 on the tick it began.
    pub hold: Option<u32>,
    /// The current step's hold has released, so its release attack, and its recovery, never
    /// begins a hold again; a step taken next clears it.
    pub released: bool,
    /// A press by hand took a step in this combo: auto mode waits, and the combo
    /// becomes the auto combo when it resets, unless it began in the dash branch.
    pub hand: bool,
    /// The auto combo auto mode plays: its presses from the root, the first in bit 0 (1 for
    /// `smash`), and how many; at most the tree's depth.
    pub auto_path: u32,
    pub auto_depth: u8,
    /// The charge level each step of the combo was released at, 4 bits a step from
    /// the root, the first in the low bits (a charge has at most `MAX_CHARGE_LEVELS` = 8
    /// levels): by hand, the level the player let go at; for a step auto mode played, the level
    /// it played. 0 for a plain step.
    pub levels: u32,
    /// The same for the auto combo: the level auto mode charges each of its steps to.
    pub auto_levels: u32,
    /// The current charge is auto mode's: it needs no button and is never perfect.
    pub charge_auto: bool,
}

/// The Sub weapon's pull on what a combo part hits, carried by the attack and the
/// orbs it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
}

/// What a smash (a step reached by B) does to the enemy it hits, carried by
/// the attack and the orbs it spawns, as a combo step's hits carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Smash {
    Pull(Pull),
    /// The enemy is pushed away from the main character, as far and as long as the pull's numbers.
    Knockback(Pull),
    /// The `bleed` status, with the bleed rune's numbers from the runes file.
    Bleed,
    /// The `stun` status of a smash, for its ticks.
    Stun {
        ticks: u32,
    },
}

/// The stage skill held: its place in the stage skills file, its level from 1, and
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

#[derive(Debug)]
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
    /// Copied from the stage when it appears.
    pub defence: u32,
    pub statuses: Statuses,
    /// The phase being carried out, 0-based (ADR-SIM-4 tier 2).
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
    /// and so does the announcement of that call.
    pub age: u32,
}

impl Clone for BossState {
    fn clone(&self) -> BossState {
        let BossState {
            id,
            boss,
            origin,
            no_call,
            kind,
            at,
            hp,
            max_hp,
            defence,
            statuses,
            phase,
            phase_age,
            invulnerable,
            step,
            wait_left,
            order,
            rules,
            parts,
            elites_called,
            age,
        } = self;
        BossState {
            id: *id,
            boss: *boss,
            origin: *origin,
            no_call: *no_call,
            kind: *kind,
            at: *at,
            hp: *hp,
            max_hp: *max_hp,
            defence: *defence,
            statuses: *statuses,
            phase: *phase,
            phase_age: *phase_age,
            invulnerable: *invulnerable,
            step: *step,
            wait_left: *wait_left,
            order: *order,
            rules: *rules,
            parts: parts.clone(),
            elites_called: *elites_called,
            age: *age,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &BossState) {
        let BossState {
            id,
            boss,
            origin,
            no_call,
            kind,
            at,
            hp,
            max_hp,
            defence,
            statuses,
            phase,
            phase_age,
            invulnerable,
            step,
            wait_left,
            order,
            rules,
            parts,
            elites_called,
            age,
        } = self;
        let BossState {
            id: s_id,
            boss: s_boss,
            origin: s_origin,
            no_call: s_no_call,
            kind: s_kind,
            at: s_at,
            hp: s_hp,
            max_hp: s_max_hp,
            defence: s_defence,
            statuses: s_statuses,
            phase: s_phase,
            phase_age: s_phase_age,
            invulnerable: s_invulnerable,
            step: s_step,
            wait_left: s_wait_left,
            order: s_order,
            rules: s_rules,
            parts: s_parts,
            elites_called: s_elites_called,
            age: s_age,
        } = source;
        *id = *s_id;
        *boss = *s_boss;
        *origin = *s_origin;
        *no_call = *s_no_call;
        *kind = *s_kind;
        *at = *s_at;
        *hp = *s_hp;
        *max_hp = *s_max_hp;
        *defence = *s_defence;
        *statuses = *s_statuses;
        *phase = *s_phase;
        *phase_age = *s_phase_age;
        *invulnerable = *s_invulnerable;
        *step = *s_step;
        *wait_left = *s_wait_left;
        *order = *s_order;
        *rules = *s_rules;
        parts.clone_from(s_parts);
        *elites_called = *s_elites_called;
        *age = *s_age;
    }
}

/// A breakable part of the boss, with its own hit points (ADR-CONT-7).
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
    /// Copied from the stage when it appears.
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

/// A companion or an enemy: a movement and a rule list (ADR-SIM-4 tier 1).
#[derive(Debug)]
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
    /// Copied from the stage when it appears.
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
    /// pixel, so it does not take its own step to its place this tick (ADR-SIM-5).
    pub crowded: bool,
    /// Enemies only: it removed itself with a `self_destruct` rule. It is at 0 hit points for
    /// the removal, but it was not defeated, so nothing credits it. It only lives until the end
    /// of the tick it is set in.
    pub burst: bool,
    /// Ticks left in which a new stun cannot land on this enemy, from the end of its last stun
    ///; 0 for none.
    pub stun_immune: u32,
    /// An elite's own state; `None` for every other agent.
    pub elite: Option<EliteState>,
    /// An agent that leaves the map unkilled (a goblin, or any elite whose `boss_call` is
    /// `leave`); `None` for every other agent.
    pub leave: Option<LeaveState>,
    /// A shrine's own state; `None` for every other agent. Boxed: almost no agent has one.
    pub shrine: Option<Box<ShrineState>>,
}

impl Clone for AgentState {
    fn clone(&self) -> AgentState {
        let AgentState {
            id,
            def,
            kind,
            key,
            at,
            hp,
            max_hp,
            defence,
            age,
            rules,
            statuses,
            aggro,
            order,
            orbit_angle,
            crowded,
            burst,
            stun_immune,
            elite,
            leave,
            shrine,
        } = self;
        AgentState {
            id: *id,
            def: *def,
            kind: *kind,
            key: *key,
            at: *at,
            hp: *hp,
            max_hp: *max_hp,
            defence: *defence,
            age: *age,
            rules: *rules,
            statuses: *statuses,
            aggro: *aggro,
            order: *order,
            orbit_angle: *orbit_angle,
            crowded: *crowded,
            burst: *burst,
            stun_immune: *stun_immune,
            elite: *elite,
            leave: *leave,
            shrine: shrine.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &AgentState) {
        let AgentState {
            id,
            def,
            kind,
            key,
            at,
            hp,
            max_hp,
            defence,
            age,
            rules,
            statuses,
            aggro,
            order,
            orbit_angle,
            crowded,
            burst,
            stun_immune,
            elite,
            leave,
            shrine,
        } = self;
        let AgentState {
            id: s_id,
            def: s_def,
            kind: s_kind,
            key: s_key,
            at: s_at,
            hp: s_hp,
            max_hp: s_max_hp,
            defence: s_defence,
            age: s_age,
            rules: s_rules,
            statuses: s_statuses,
            aggro: s_aggro,
            order: s_order,
            orbit_angle: s_orbit_angle,
            crowded: s_crowded,
            burst: s_burst,
            stun_immune: s_stun_immune,
            elite: s_elite,
            leave: s_leave,
            shrine: s_shrine,
        } = source;
        *id = *s_id;
        *def = *s_def;
        *kind = *s_kind;
        *key = *s_key;
        *at = *s_at;
        *hp = *s_hp;
        *max_hp = *s_max_hp;
        *defence = *s_defence;
        *age = *s_age;
        *rules = *s_rules;
        *statuses = *s_statuses;
        *aggro = *s_aggro;
        *order = *s_order;
        *orbit_angle = *s_orbit_angle;
        *crowded = *s_crowded;
        *burst = *s_burst;
        *stun_immune = *s_stun_immune;
        *elite = *s_elite;
        *leave = *s_leave;
        shrine.clone_from(s_shrine);
    }
}

impl AgentState {
    /// A mimic that has not revealed yet: an object nothing touches.
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

/// A shrine's fight.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ShrineState {
    /// Ticks since the main character touched it, 0 on the tick it did; `None` while untouched.
    pub fight: Option<u32>,
    /// The guardians that spawned, in ring order.
    pub guardians: Vec<EntityId>,
}

impl Clone for ShrineState {
    fn clone(&self) -> ShrineState {
        let ShrineState { fight, guardians } = self;
        ShrineState {
            fight: *fight,
            guardians: guardians.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ShrineState) {
        let ShrineState { fight, guardians } = self;
        let ShrineState {
            fight: s_fight,
            guardians: s_guardians,
        } = source;
        *fight = *s_fight;
        guardians.clone_from(s_guardians);
    }
}

/// When an elite that leaves is to go.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct LeaveState {
    /// Ticks since it was disturbed, 0 on the tick it was; `None` while it is not.
    pub disturbed: Option<u32>,
    /// A boss came or called: it goes on this tick.
    pub go: bool,
}

/// An elite holds `home`, where it spawned, until a boss calls it.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct EliteState {
    pub home: Point,
    pub called: bool,
    /// What the boss's call does to it; its definition's, so not part of the hash.
    pub boss_call: BossCall,
    /// A mimic that has not revealed yet; false for every other elite.
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
/// A path into an attack program: one step for each level of nesting, and a hooked orb sits at
/// depth `MAX_PATTERN_DEPTH - 1` at the deepest (its burst is one level below), so the path is
/// never longer than `MAX_PATTERN_DEPTH`. Inline, so that an orb's hook allocates nothing (T1b).
pub(crate) type Path = InlineVec<PathStep, { validate::limits::MAX_PATTERN_DEPTH as usize }>;

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct RuleState {
    pub cooldown_left: u32,
    /// Firings left; `UNLIMITED` if there is no limit.
    pub charges_left: u32,
    pub fired: bool,
}

/// The statuses of this version (ADR-SIM-4): the three a skill can apply, and bleed, poison and
/// burn, which only their runes apply. The IDs in the registry are the names of the
/// first three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// More damage dealt.
    AtkUp,
    /// More damage taken.
    Vulnerable,
    /// Slower movement.
    Slow,
    /// Damage over time, in stacks.
    Bleed,
    /// Damage over time, in stacks, of its own beside bleed's.
    Poison,
    /// Damage over time, in stacks, of its own beside bleed's.
    Burn,
    /// The enemy stops what it was doing for its ticks. Not a skill's status.
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
    /// The ticks of the application that set `remaining`, for the snapshot: a
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

/// What a damage-over-time status deals and when: its damage a stack
/// and its type, from the content that applied it; the atk-up and type bonus of its source,
/// fixed when it was last applied, as an attack's are when it starts; its interval,
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
/// character's combo steps aim along its facing. A new way of aiming (an effect's
/// target) is a new variant, read in `attacks::aim`, hashed in its `StableHash` impl and given its
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
    /// The type of the phase that fired it; `None` is neutral.
    pub damage_type: Option<DamageType>,
    /// Its hit breaks the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Fired by a combo step, A or B: its hit adds expertise points.
    pub combo: bool,
    /// Starts an attack of its own when it ends: an orb's burst.
    pub hook: Option<Hook>,
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
#[derive(Debug)]
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

impl Clone for ZoneState {
    fn clone(&self) -> ZoneState {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            wind_ticks,
            active_ticks,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
            program,
        } = self;
        ZoneState {
            owner: *owner,
            friendly: *friendly,
            shape: shape.clone(),
            at: *at,
            dir: *dir,
            wind_left: *wind_left,
            active_left: *active_left,
            wind_ticks: *wind_ticks,
            active_ticks: *active_ticks,
            damage: *damage,
            damage_type: *damage_type,
            breaks_charge: *breaks_charge,
            warn_only: *warn_only,
            hits: hits.clone(),
            program: *program,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ZoneState) {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            wind_ticks,
            active_ticks,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
            program,
        } = self;
        let ZoneState {
            owner: s_owner,
            friendly: s_friendly,
            shape: s_shape,
            at: s_at,
            dir: s_dir,
            wind_left: s_wind_left,
            active_left: s_active_left,
            wind_ticks: s_wind_ticks,
            active_ticks: s_active_ticks,
            damage: s_damage,
            damage_type: s_damage_type,
            breaks_charge: s_breaks_charge,
            warn_only: s_warn_only,
            hits: s_hits,
            program: s_program,
        } = source;
        *owner = *s_owner;
        *friendly = *s_friendly;
        shape.clone_from(s_shape);
        *at = *s_at;
        *dir = *s_dir;
        *wind_left = *s_wind_left;
        *active_left = *s_active_left;
        *wind_ticks = *s_wind_ticks;
        *active_ticks = *s_active_ticks;
        *damage = *s_damage;
        *damage_type = *s_damage_type;
        *breaks_charge = *s_breaks_charge;
        *warn_only = *s_warn_only;
        hits.clone_from(s_hits);
        *program = *s_program;
    }
}

/// An orb's burst, waiting for the orb to end.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Hook {
    pub program: ProgramIndex,
    /// The `orb` node in the program: its `burst` list is what the orb starts.
    pub path: Path,
    /// Ticks left before it starts.
    pub left: u32,
    pub owner: EntityId,
    pub aim: Aim,
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Of a combo step, A or B.
    pub combo: bool,
}

/// One step down a program: the node at `node` in the current list, then its `body` list, or
/// its `burst` list if `then` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// A phased attack an attacker is carrying out: one at a time per attacker and slot.
/// Its phases spawn their nodes as `AttackTask`s, which run to their own end.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackRun {
    pub owner: EntityId,
    /// 0 for an agent's or the boss's attacks and the main character's combo.
    pub slot: u8,
    /// The attack it carries out, compiled when the run started (`World::attacks`).
    pub attack: AttackIndex,
    /// The attack speed that shortens its phases, in percent, and the range that widens its hit
    /// boxes, fixed when it starts; read through `attacks::phase`.
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
    /// The damage its starter gives a phase that sets none: the main character's
    /// attack, a shot skill's own damage, 1 for an enemy or the boss.
    pub base: u32,
    /// The attacker's atk-up, in percent, fixed when the attack starts.
    pub atk_up_pct: u32,
    /// The percent of the damage a difficulty scales an enemy's or the boss's attack by
    /// fixed when the attack starts; 100 for the main character's and its allies'.
    pub damage_pct: u32,
    /// The attacker's type bonus for each damage type, by its place in `DamageType::ALL`, in
    /// percent, each at most the match-up file's `max_type_bonus_pct`, fixed when the attack
    /// starts: the main character's, once it holds any; none for every other
    /// attacker. Held inline: boxing it allocated on every launch (T1b).
    pub type_bonus_pct: Option<[u32; DamageType::ALL.len()]>,
    /// `phase_time` of the attack's first tick, for the nodes its phases spawn.
    pub age: u32,
    /// Who the current active phase has hit, and on which tick last, sorted by entity ID;
    /// emptied when an active phase begins.
    pub hits: Vec<(EntityId, u32)>,
    /// The Sub weapon's pull, when the attack is a combo part.
    pub smash: Option<Smash>,
    /// Where its hit boxes are centred instead of on its attacker: an on-hit rune's attack, at
    /// the point the combo step hit, or where a thrown one has flown to.
    pub at: Option<Point>,
    /// A thrown rune's turn point, the point the combo step hit: its first phase flies `at`
    /// there from the main character, its second back to the main character (boomerang). A
    /// homing missile's goal: where its target is, or was last.
    pub throw: Option<Point>,
    /// A homing missile's target: `throw` follows it while it is alive, and the
    /// missile is gone once it hits anything.
    pub homing: Option<EntityId>,
    /// Where its target stood when it began, for the hit boxes and zones anchored at the target
    /// (core Z1); only an attack that has one holds it.
    pub spot: Option<Point>,
}

impl Clone for AttackRun {
    fn clone(&self) -> AttackRun {
        let AttackRun {
            owner,
            slot,
            attack,
            speed_pct,
            width_pct,
            phase,
            tick,
            facing,
            aim,
            friendly,
            ranged,
            base,
            atk_up_pct,
            damage_pct,
            type_bonus_pct,
            age,
            hits,
            smash,
            at,
            throw,
            homing,
            spot,
        } = self;
        AttackRun {
            owner: *owner,
            slot: *slot,
            attack: *attack,
            speed_pct: *speed_pct,
            width_pct: *width_pct,
            phase: *phase,
            tick: *tick,
            facing: *facing,
            aim: *aim,
            friendly: *friendly,
            ranged: *ranged,
            base: *base,
            atk_up_pct: *atk_up_pct,
            damage_pct: *damage_pct,
            type_bonus_pct: *type_bonus_pct,
            age: *age,
            hits: hits.clone(),
            smash: *smash,
            at: *at,
            throw: *throw,
            homing: *homing,
            spot: *spot,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &AttackRun) {
        let AttackRun {
            owner,
            slot,
            attack,
            speed_pct,
            width_pct,
            phase,
            tick,
            facing,
            aim,
            friendly,
            ranged,
            base,
            atk_up_pct,
            damage_pct,
            type_bonus_pct,
            age,
            hits,
            smash,
            at,
            throw,
            homing,
            spot,
        } = self;
        let AttackRun {
            owner: s_owner,
            slot: s_slot,
            attack: s_attack,
            speed_pct: s_speed_pct,
            width_pct: s_width_pct,
            phase: s_phase,
            tick: s_tick,
            facing: s_facing,
            aim: s_aim,
            friendly: s_friendly,
            ranged: s_ranged,
            base: s_base,
            atk_up_pct: s_atk_up_pct,
            damage_pct: s_damage_pct,
            type_bonus_pct: s_type_bonus_pct,
            age: s_age,
            hits: s_hits,
            smash: s_smash,
            at: s_at,
            throw: s_throw,
            homing: s_homing,
            spot: s_spot,
        } = source;
        *owner = *s_owner;
        *slot = *s_slot;
        *attack = *s_attack;
        *speed_pct = *s_speed_pct;
        *width_pct = *s_width_pct;
        *phase = *s_phase;
        *tick = *s_tick;
        *facing = *s_facing;
        *aim = *s_aim;
        *friendly = *s_friendly;
        *ranged = *s_ranged;
        *base = *s_base;
        *atk_up_pct = *s_atk_up_pct;
        *damage_pct = *s_damage_pct;
        *type_bonus_pct = *s_type_bonus_pct;
        *age = *s_age;
        hits.clone_from(s_hits);
        *smash = *s_smash;
        *at = *s_at;
        *throw = *s_throw;
        *homing = *s_homing;
        *spot = *s_spot;
    }
}

/// The movement speed, in percent, that the running attacks leave each attacker: the lowest
/// `move_pct` of the current phases of its runs. Kept beside `State::runs` so that asking costs
/// a search over the attackers, not a scan of every run; a derived value that no hash reads.
/// Sorted by owner, one entry for each owner with a run.
#[derive(Debug, Default)]
pub(crate) struct MoveIndex {
    by_owner: Vec<(EntityId, i64)>,
}

impl Clone for MoveIndex {
    fn clone(&self) -> MoveIndex {
        let MoveIndex { by_owner } = self;
        MoveIndex {
            by_owner: by_owner.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &MoveIndex) {
        let MoveIndex { by_owner } = self;
        let MoveIndex {
            by_owner: s_by_owner,
        } = source;
        by_owner.clone_from(s_by_owner);
    }
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

/// An attack being carried out (ADR-SIM-4 tier 3): a program, where it has got to, and what it
/// needs to know to aim and to hurt. Presets are expanded before they get here, so a preset and
/// its inline copy are the same task.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackTask {
    pub owner: EntityId,
    pub program: ProgramIndex,
    /// Where in the program this task starts; empty for the whole program.
    pub base: Path,
    pub origin: Origin,
    /// What `aimed` directions point at.
    pub aim: Aim,
    pub friendly: bool,
    pub damage: u32,
    /// The type of the phase that started it, which its orbs carry.
    pub damage_type: Option<DamageType>,
    /// The orbs it fires break the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Started by a combo step, A or B: its hits add expertise points.
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
