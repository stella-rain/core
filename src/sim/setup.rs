//! The run's setup of the live rules: the pool, the difficulty, the initial state and the boss state.

use super::{
    BossIndex, MAX_ATTACK_TASKS, MAX_ORBS_ALIVE, MAX_RANGED_ATTACKERS, MAX_ZONES_ALIVE, Scratch,
    World, agents, attacks, boss, equipment, expertise, player, runes, scratch,
};
use crate::attack::DamageType;
use crate::difficulties::scale_pct;
use crate::engine::Outcome;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::kinds::KindTable;
use crate::rewards::Rarity;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::sim::state::{
    AgentKey, AgentSource, BossState, Career, Dir, Offers, PartState, PlayerCombat, PlayerState,
    Progress, Stagger, StarField, StatRule, State, Streams,
};
use crate::sim::state::{MAX_PLAYERS, PlayerIdx, Players};
use crate::trig;

/// What a run brings to its offers: the rewards of its pool, by their place in the rewards file
/// (ADR-REPL-5).
pub(crate) struct RunPool {
    pub rewards: Vec<u16>,
    pub difficulty: RunDifficulty,
}

/// `hp` scaled by `pct`: never to 0 from a positive value, which would be dead at
/// its spawn.
pub(crate) fn scale_hp(hp: u32, pct: u32) -> u32 {
    match hp {
        0 => 0,
        _ => scale_pct(hp, pct).max(1),
    }
}

/// What a run takes from its difficulty: the percents its enemies' hit points and
/// attacks' damage are scaled by, and the range of rarities its stars and offers may have.
/// Fixed for the run, part of the setup, and hashed with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunDifficulty {
    pub hp_pct: u32,
    pub damage_pct: u32,
    pub rarity: (Rarity, Rarity),
}

impl RunDifficulty {
    /// A stage run alone, without a setup: nothing scaled and no rarity left out.
    pub const NONE: RunDifficulty = RunDifficulty {
        hp_pct: 100,
        damage_pct: 100,
        rarity: (Rarity::Common, Rarity::Supernova),
    };
}

/// The state before the first tick, its generator seeded with `seed`: the stage's own for a
/// stage, the setup's for a run (ADR-REPL-5).
pub(crate) fn initial_state(
    stage: &World,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
    kinds: KindTable,
) -> State {
    let mut state = unreserved_state(stage, content, pool, seed, kinds);
    // The weapon's base set at level 1, for slot 0; a party member or a joiner copies the career.
    expertise::start_set(stage, content.expertise(), &mut state, PlayerIdx::SOLO);
    reserve(stage, &mut state);
    state
}

/// Makes room, once, for the most the run's per-enemy lists can hold (DIR1, ADR-SIM-5 point 4): the cap on enemies alive, and the other caps, so that no tick grows a buffer.
/// Called when the run is built and again for a clone of its state, because a cloned `Vec` has
/// only its length. The scratch buffers are `Scratch::reserve`'s.
pub(crate) fn reserve(stage: &World, s: &mut State) {
    let cap = stage.enemy_cap as usize;
    let map = stage.map();
    s.scratch.reserve(cap, map.width.0, map.height.0);
    scratch::reserve_to(&mut s.enemies, cap);
    s.move_index
        .reserve(cap + crate::validate::limits::MAX_COMPANIONS + 1);
    scratch::reserve_to(&mut s.tasks, MAX_ATTACK_TASKS as usize);
    scratch::reserve_to(
        &mut s.runs,
        cap + crate::validate::limits::MAX_COMPANIONS + 1,
    );
    scratch::reserve_to(&mut s.ranged_cooling, MAX_RANGED_ATTACKERS as usize);
    scratch::reserve_to(&mut s.orbs, MAX_ORBS_ALIVE as usize);
    scratch::reserve_to(&mut s.zones, MAX_ZONES_ALIVE as usize);
    if let (Some(cfg), Some(f)) = (&stage.stars, s.stars.as_mut()) {
        let stars = usize::from(cfg.cap.min(crate::validate::limits::MAX_STARS_ON_MAP));
        scratch::reserve_to(&mut f.on_map, stars);
    }
    if let (Some(cfg), Some(pickups)) = (&stage.heal, s.heal_pickups.as_mut()) {
        scratch::reserve_to(pickups, usize::from(cfg.cap));
    }
    // Each present slot's inventory has room for its bound, so a drop allocates nothing
    // (ADR-SIM-5 points 7 and 8).
    for me in s.players.indices() {
        let inventory = &mut s.players.career_mut(me).equipment.inventory;
        scratch::reserve_to(inventory, crate::limits::MAX_INVENTORY);
    }
}

/// Events a tick reserves room for per enemy alive (hits, kills, spawns, bursts).
const EVENTS_PER_ENEMY: usize = 4;
/// Events a tick reserves room for beyond those.
const EVENTS_SPARE: usize = 64;

/// How far apart, in degrees, the orbiting companions start.
const COMPANION_SPACING_DEGREES: i32 = 90;

/// The room for a tick's events (DIR1): a few per enemy alive (hits, kills, spawns, bursts) and
/// a few more. A placeholder bound: a tick with more grows the list once, and `tests/alloc.rs`
/// checks the runs it measures stay under it.
pub(crate) fn events_room(stage: &World) -> usize {
    stage.enemy_cap as usize * EVENTS_PER_ENEMY + EVENTS_SPARE
}

fn unreserved_state(
    stage: &World,
    content: &RunContent,
    pool: RunPool,
    seed: u64,
    kinds: KindTable,
) -> State {
    let run_stats = content.run_stats();
    // Slots 0 to 2 are reserved for the players whether or not a slot is in the party
    // (ADR-SIM-7); the boss and the companions come after them.
    let mut next_id = u32::from(MAX_PLAYERS);
    // A boss with an arrival tick after the first is built on that tick (`boss::arrive`).
    let boss_id = stage
        .boss
        .as_ref()
        .filter(|b| boss::on_field_at_start(b))
        .map(|_| {
            let id = EntityId(next_id);
            next_id += 1;
            id
        });
    let start = stage.map().start();
    let companions = stage
        .companions
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let id = EntityId(next_id);
            next_id += 1;
            let key = AgentKey {
                source: AgentSource::Companion,
                index: u32::try_from(i).unwrap_or(u32::MAX),
                rule: 0,
            };
            let index = stage.defs.of_key(key);
            // Orbiting companions start a quarter turn apart.
            let angle =
                i32::try_from(i).unwrap_or(0) * COMPANION_SPACING_DEGREES * trig::PER_DEGREE;
            agents::new_agent(id, index, stage.defs.get(index), key, start, angle)
        })
        .collect();
    // The boss's parts come after the companions in the order of entity IDs. Of these, the crit roll
    // and the push-apart angle read a number: they read `State::roll_key`, the number without
    // the empty player slots (ADR-SIM-7).
    let boss = boss_id.map(|id| {
        let arrival = Arrival {
            at: stage.boss(BossIndex(0)).boss.spawn,
            origin: Point::default(),
            no_call: false,
            mid_pct: None,
        };
        boss_state(
            stage,
            BossIndex(0),
            id,
            &mut next_id,
            pool.difficulty.hp_pct,
            arrival,
            &kinds,
        )
    });
    let progress = stage.levels.as_ref().map(|_| Progress {
        level: 1,
        xp: 0,
        stats: run_stats
            .stats()
            .iter()
            .map(|s| StatRule {
                id: s.id.clone(),
                effect: s.effect,
                step: s.step,
                limit: s.limit,
            })
            .collect(),
        priority: (0..run_stats.stats().len())
            .map(|i| u8::try_from(i).unwrap_or(u8::MAX))
            .collect(),
        spent: vec![0; run_stats.stats().len()],
        totals: [0; StatEffect::ALL.len()],
        set_tenths: [0; StatEffect::ALL.len()],
    });
    let offers = stage.stars.as_ref().map(|_| Offers {
        set_counts: vec![0; content.constellations().sets().len()],
    });
    State {
        scratch: Scratch::default(),
        tick: 0,
        rng: Streams::new(seed),
        outcome: Outcome::Running,
        players: Players::solo(
            PlayerState {
                at: start,
                hp: stage.player.hp,
                max_hp: stage.player.hp,
                defence: stage.player.defence,
                invulnerable: 0,
                // Up the map unless the map's facing says otherwise (core#147).
                facing: stage
                    .map()
                    .facing
                    .map_or(Dir::UP, |angle| attacks::rotate(Dir::DOWN, angle)),
                target: None,
                // The validator holds it to 1..=3; the clamp keeps an unvalidated stage there too.
                evade_charges: stage
                    .player
                    .evade
                    .charges
                    .clamp(1, crate::validate::limits::MAX_EVADE_CHARGES),
                evade_recovering: Vec::new(),
                held: 0,
                combo: player::initial_combo(stage),
                dash: None,
                skill: None,
                pull: None,
                shield: None,
                orbits: Vec::new(),
                range_pct: 0,
                rune_haste_pct: 0,
                pickup_radius_pct: 0,
                skill_cooldown_pct: 0,
                kill_heal: 0,
                kill_heal_every: 0,
                kills_counted: 0,
                crit_chance_pct: 0,
                crit_damage_pct: 0,
                echo_hit_pct: 0,
                burst_pct: 0,
                burst_radius: 0,
                burst_depth: 0,
                burst_depth_extra: 0,
                type_bonus_pct: [0; DamageType::ALL.len()],
                runes: Vec::new(),
                aggro: 0,
                statuses: Default::default(),
                down_ticks: 0,
                revive_hold: 0,
                out: false,
                combat: PlayerCombat::default(),
            },
            Career::new(
                seed,
                // The run's first player; the other slots are built by the party builder (a test
                // here, recording parties later).
                PlayerIdx::SOLO,
                progress,
                offers,
                expertise::initial(stage, content.expertise()),
                stage.stardust.as_ref().map(|_| 0),
                equipment::Equipment::default(),
            ),
        ),
        boss,
        companions,
        enemies: Vec::new(),
        tasks: Vec::new(),
        runs: Vec::new(),
        move_index: crate::sim::state::MoveIndex::default(),
        ranged_cooling: Vec::new(),
        orbs: Vec::new(),
        zones: Vec::new(),
        spawned: vec![0; stage.waves.len()],
        next_id,
        next_orb_group: 0,
        stars: stage.stars.as_ref().map(|_| StarField {
            misses: 0,
            on_map: Vec::new(),
            rarity: pool.difficulty.rarity,
            rune_pool: runes::FULL_POOL,
            pool: pool.rewards,
        }),
        heal_pickups: stage.heal.as_ref().map(|_| Vec::new()),
        match_ups: *content.match_ups(),
        runes: std::sync::Arc::new(content.runes().clone()),
        stun: run_stats.stun(),
        max_attack_speed_pct: run_stats.max_attack_speed_pct(),
        enemy_attack_damage: run_stats.enemy_attack_damage(),
        max_rune_haste_pct: content.rewards().max_rune_haste_pct(),
        horde: None,
        hp_pct: pool.difficulty.hp_pct,
        damage_pct: pool.difficulty.damage_pct,
        crit_seed: seed,
        echoes: Vec::new(),
        repeats: Vec::new(),
        kinds: std::sync::Arc::new(kinds),
    }
}

/// The big patterns the boss has announced, for the snapshot; an enemy's are not.
pub(crate) fn announced_patterns(stage: &World, s: &State) -> Vec<crate::snapshot::PatternView> {
    boss::announced(stage, s)
}

/// What differs between the arrivals of one boss definition: where it stands, how far its
/// `move_to` targets are moved, whether it calls elites, and the percent its hit points and its
/// parts' are scaled by first (a `time_limit` mid-boss's, core#336).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Arrival {
    pub at: Point,
    pub origin: Point,
    pub no_call: bool,
    pub mid_pct: Option<u32>,
}

/// The boss before its first tick, its parts taking the entity IDs from `next_id` on. The
/// first phase begins on the first tick (see `boss::act`).
/// The boss's state at its spawn, its hit points and its parts' scaled by `hp_pct`.
pub(crate) fn boss_state(
    stage: &World,
    boss: BossIndex,
    id: EntityId,
    next_id: &mut u32,
    hp_pct: u32,
    arrival: Arrival,
    kinds: &KindTable,
) -> BossState {
    let def = stage.boss(boss);
    let b = &def.boss;
    let hp = |hp: u32| {
        let hp = arrival.mid_pct.map_or(hp, |pct| scale_hp(hp, pct));
        scale_hp(hp, hp_pct)
    };
    BossState {
        id,
        boss,
        origin: arrival.origin,
        no_call: arrival.no_call,
        kind: def.kind,
        at: arrival.at,
        hp: hp(b.hp),
        max_hp: hp(b.hp),
        defence: b.defence,
        statuses: Default::default(),
        phase: 0,
        phase_age: 0,
        invulnerable: 0,
        step: 0,
        wait_left: 0,
        order: None,
        rules: Default::default(),
        // A boss that calls no elites hashes nothing for it, a no-call boss too.
        elites_called: b.call_elites.filter(|_| !arrival.no_call).map(|_| false),
        age: 0,
        stagger: Stagger::default(),
        parts: b
            .parts
            .iter()
            .map(|p| {
                let part_id = EntityId(*next_id);
                *next_id += 1;
                PartState {
                    id: part_id,
                    name: p.id.clone(),
                    kind: kinds.of(&p.id),
                    hp: hp(p.hp),
                    max_hp: hp(p.hp),
                    defence: p.defence,
                    offset: p.offset,
                    radius: p.radius.0,
                    statuses: Default::default(),
                }
            })
            .collect(),
    }
}
