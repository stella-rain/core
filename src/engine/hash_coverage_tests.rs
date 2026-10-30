//! Every field of the run's state moves the state hash, or is listed here as one that does not,
//! with its reason (ADR-SIM-3 point 2). Each state type has one invocation of
//! `each_field_moves_the_hash!`, which destructures the type without `..`: a field that is in
//! none of its three lists does not compile.

use super::*;
use crate::attack::{DamageType, ZoneShape};
use crate::behaviour::BossCall;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::kinds::KindIndex;
use crate::pack::ContentFiles;
use crate::rewards::Rarity;
use crate::run_content::RunContent;
use crate::rune::Rune;
use crate::sim::{AttackIndex, BossIndex, DefIndex, ProgramIndex, RangedPool};
use crate::snapshot::{ExpertiseOfferKind, OfferKind};
use crate::time_limit::TimeLimit;
use crate::version::Versions;

/// Changes a value to another one of its type, so that its owner's hash has to move if the
/// field holding it is hashed.
pub(super) trait Perturb {
    fn perturb(&mut self);
}

macro_rules! perturb_ints {
    ($($t:ty),*) => {$(
        impl Perturb for $t {
            fn perturb(&mut self) {
                *self = self.wrapping_add(1);
            }
        }
    )*};
}
perturb_ints!(u8, u16, u32, u64, i32, i64);

impl Perturb for bool {
    fn perturb(&mut self) {
        *self = !*self;
    }
}

impl Perturb for Fx {
    fn perturb(&mut self) {
        self.0.perturb();
    }
}

impl<T: Perturb> Perturb for Option<T> {
    fn perturb(&mut self) {
        // The base holds every `Option` as `Some`, so the change is to `None`.
        assert!(
            self.is_some(),
            "the base state must hold every Option as Some"
        );
        *self = None;
    }
}

impl<T: Clone> Perturb for Vec<T> {
    fn perturb(&mut self) {
        let first = self
            .first()
            .expect("the base state must hold every list non-empty")
            .clone();
        self.push(first);
    }
}

impl<T: Copy + Default, const N: usize> Perturb for crate::inline::InlineVec<T, N> {
    fn perturb(&mut self) {
        self.push(T::default());
    }
}

impl<T: Perturb> Perturb for Box<T> {
    fn perturb(&mut self) {
        (**self).perturb();
    }
}

impl<T: Perturb, const N: usize> Perturb for [T; N] {
    fn perturb(&mut self) {
        self[0].perturb();
    }
}

impl<A: Perturb, B> Perturb for (A, B) {
    fn perturb(&mut self) {
        self.0.perturb();
    }
}

impl Perturb for ContentId {
    fn perturb(&mut self) {
        *self = ContentId::from(format!("{self}x"));
    }
}

/// A struct is perturbed through one of its hashed fields; a newtype through its number.
macro_rules! perturb_through {
    ($($t:ty => $f:tt),* $(,)?) => {$(
        impl Perturb for $t {
            fn perturb(&mut self) {
                self.$f.perturb();
            }
        }
    )*};
}
perturb_through! {
    EntityId => 0,
    KindIndex => 0,
    DefIndex => 0,
    BossIndex => 0,
    ProgramIndex => 0,
    Point => x,
    Dir => x,
    Pull => distance,
    PathStep => node,
    Frame => idx,
    AgentKey => index,
    ComboState => path,
    SkillSlot => index,
    OrbitState => reward,
    HeldRune => cooldown_left,
    PullState => ticks_left,
    ShieldState => amount,
    DashState => done,
    MoveOrder => ticks_left,
    RuleState => cooldown_left,
    StatusState => remaining,
    DotState => damage,
    Echo => damage,
    HealPickup => id,
    StarState => id,
    Due => rarity,
    OfferState => rarity,
    ExpertiseOffer => level,
    ShrineState => guardians,
    LeaveState => go,
    EliteState => called,
    OrbLink => group,
    Hook => left,
    StatRule => step,
    Streams => spawn,
    PlayerState => at,
    BossState => hp,
    PartState => hp,
    AgentState => hp,
    AttackTask => damage,
    AttackRun => base,
    OrbState => damage,
    ZoneState => damage,
    Progress => xp,
    StarField => misses,
    ExpertiseGauge => points,
    Horde => spawned,
}

impl Perturb for crate::rng::SplitMix64 {
    fn perturb(&mut self) {
        self.next_u64();
    }
}

impl Perturb for AttackIndex {
    fn perturb(&mut self) {
        // Flips between the first two attacks, which every run's stage has.
        self.0 = 1 - self.0.min(1);
    }
}

macro_rules! perturb_cycle {
    ($t:ty, $all:expr) => {
        impl Perturb for $t {
            fn perturb(&mut self) {
                let all = $all;
                let at = all.iter().position(|v| v == self).expect("listed");
                *self = all[(at + 1) % all.len()];
            }
        }
    };
}
perturb_cycle!(
    Outcome,
    [Outcome::Running, Outcome::Cleared, Outcome::Failed]
);
perturb_cycle!(DamageType, DamageType::ALL);
perturb_cycle!(StatusKind, StatusKind::ALL);
perturb_cycle!(RangedPool, RangedPool::ALL);
perturb_cycle!(
    Rarity,
    [
        Rarity::Common,
        Rarity::Uncommon,
        Rarity::Rare,
        Rarity::Epic,
        Rarity::Legendary,
        Rarity::Supernova
    ]
);
perturb_cycle!(OfferKind, [OfferKind::Star, OfferKind::Boss]);
perturb_cycle!(
    ExpertiseOfferKind,
    [
        ExpertiseOfferKind::Common,
        ExpertiseOfferKind::Style,
        ExpertiseOfferKind::Upgrade,
        ExpertiseOfferKind::Capstone
    ]
);
perturb_cycle!(
    Rune,
    [
        Rune::Explosion,
        Rune::Thunder,
        Rune::Boomerang,
        Rune::HomingMissiles,
        Rune::Bleed,
        Rune::Poison,
        Rune::Burn
    ]
);
perturb_cycle!(
    AgentSource,
    [
        AgentSource::Companion,
        AgentSource::Wave,
        AgentSource::Summoned,
        AgentSource::BossRule,
        AgentSource::Guardian,
        AgentSource::Escort
    ]
);
perturb_cycle!(BossCall, [BossCall::Join, BossCall::Leave, BossCall::Stay]);

impl Perturb for Smash {
    fn perturb(&mut self) {
        *self = match self {
            Smash::Pull(_) => Smash::Bleed,
            Smash::Knockback(_) => Smash::Stun { ticks: 1 },
            Smash::Bleed => Smash::Pull(Pull {
                distance: 1,
                ticks: 1,
            }),
            Smash::Stun { .. } => Smash::Knockback(Pull {
                distance: 1,
                ticks: 1,
            }),
        };
    }
}

impl Perturb for Aim {
    fn perturb(&mut self) {
        *self = match self {
            Aim::Entity(_) => Aim::Fixed(Dir::DOWN),
            Aim::Fixed(_) => Aim::Entity(EntityId(1)),
        };
    }
}

impl Perturb for Origin {
    fn perturb(&mut self) {
        *self = match self {
            Origin::Owner => Origin::At(Point::default()),
            Origin::At(_) => Origin::Owner,
        };
    }
}

impl Perturb for ZoneShape {
    fn perturb(&mut self) {
        match self {
            ZoneShape::Circle { radius } | ZoneShape::Arc { radius, .. } => radius.perturb(),
            ZoneShape::Line { length, .. } => length.perturb(),
        }
    }
}

impl Perturb for MoveIndex {
    fn perturb(&mut self) {
        self.lower(EntityId(9999), 5);
    }
}

// The values below fill the base state: every `Option` is `Some`, every list holds an item,
// every number is distinct enough that no two fields are mixed up by the byte layout.

fn pt(n: i32) -> Point {
    Point {
        x: Fx(n),
        y: Fx(n + 1),
    }
}

fn dir(n: i32) -> Dir {
    Dir { x: n, y: -n }
}

fn status() -> StatusState {
    StatusState {
        kind: StatusKind::Bleed,
        source: EntityId(3),
        remaining: 4,
        total: 5,
        stacks: 2,
        value_pct: 7,
        dot: Some(DotState {
            damage: 1,
            damage_type: Some(DamageType::Pyro),
            atk_up_pct: 2,
            type_bonus_pct: 3,
            interval: 4,
            next_in: 5,
        }),
    }
}

fn rules() -> Rules {
    [RuleState {
        cooldown_left: 1,
        charges_left: 2,
        fired: true,
    }]
    .into_iter()
    .collect()
}

fn statuses() -> Statuses {
    [status()].into_iter().collect()
}

fn smash() -> Smash {
    Smash::Pull(Pull {
        distance: 3,
        ticks: 4,
    })
}

fn agent() -> AgentState {
    AgentState {
        id: EntityId(10),
        def: DefIndex(0),
        kind: KindIndex(0),
        key: AgentKey {
            source: AgentSource::Wave,
            index: 1,
            rule: 2,
        },
        at: pt(1),
        hp: 5,
        max_hp: 6,
        defence: 7,
        age: 8,
        rules: rules(),
        statuses: statuses(),
        aggro: 9,
        order: Some(MoveOrder {
            to: pt(2),
            ticks_left: 3,
        }),
        orbit_angle: 4,
        crowded: true,
        burst: true,
        stun_immune: 5,
        elite: Some(EliteState {
            home: pt(3),
            called: true,
            boss_call: BossCall::Leave,
            disguised: true,
        }),
        leave: Some(LeaveState {
            disturbed: Some(2),
            go: true,
        }),
        shrine: Some(Box::new(ShrineState {
            fight: Some(1),
            guardians: vec![EntityId(11)],
        })),
    }
}

fn boss() -> BossState {
    BossState {
        id: EntityId(20),
        boss: BossIndex(0),
        origin: pt(4),
        no_call: true,
        kind: KindIndex(0),
        at: pt(5),
        hp: 6,
        max_hp: 7,
        defence: 8,
        statuses: statuses(),
        phase: 1,
        phase_age: 2,
        invulnerable: 3,
        step: 4,
        wait_left: 5,
        order: Some(MoveOrder {
            to: pt(6),
            ticks_left: 7,
        }),
        rules: rules(),
        parts: vec![PartState {
            id: EntityId(21),
            name: ContentId::from("part"),
            kind: KindIndex(0),
            hp: 3,
            max_hp: 4,
            defence: 5,
            offset: pt(7),
            radius: 8,
            statuses: statuses(),
        }],
        elites_called: Some(true),
        age: 9,
    }
}

fn hook() -> Hook {
    Hook {
        program: ProgramIndex(0),
        path: vec![PathStep {
            node: 1,
            then: true,
        }],
        left: 2,
        owner: EntityId(30),
        aim: Aim::Fixed(dir(3)),
        damage: 4,
        damage_type: Some(DamageType::Cryo),
        breaks_charge: true,
        smash: Some(smash()),
        combo: true,
    }
}

fn orb() -> OrbState {
    OrbState {
        at: pt(1),
        vx: Fx(2),
        vy: Fx(3),
        dir: dir(4),
        style: 5,
        owner: EntityId(31),
        damage: 6,
        damage_type: Some(DamageType::Geo),
        breaks_charge: true,
        smash: Some(smash()),
        combo: true,
        hook: Some(Box::new(hook())),
        breakable: true,
        life: 7,
        stop: Some(8),
        link: Some(OrbLink {
            group: 1,
            slot: 2,
            of: 3,
            width: Fx(4),
            damage: 5,
            rehit_ticks: 6,
            last_hit: Some(7),
        }),
        program: ProgramIndex(9),
    }
}

fn zone() -> ZoneState {
    ZoneState {
        owner: EntityId(32),
        friendly: true,
        shape: ZoneShape::Arc {
            radius: Fx(5),
            half_angle: 6,
        },
        at: pt(1),
        dir: dir(2),
        wind_left: 3,
        active_left: 4,
        wind_ticks: 5,
        active_ticks: 6,
        damage: 7,
        damage_type: Some(DamageType::Anemo),
        breaks_charge: true,
        warn_only: true,
        hits: vec![(EntityId(1), 2)],
        program: ProgramIndex(9),
    }
}

fn task() -> AttackTask {
    AttackTask {
        owner: EntityId(33),
        program: ProgramIndex(0),
        base: vec![PathStep {
            node: 2,
            then: true,
        }],
        origin: Origin::At(pt(2)),
        aim: Aim::Entity(EntityId(3)),
        friendly: true,
        damage: 4,
        damage_type: Some(DamageType::Hydro),
        breaks_charge: true,
        smash: Some(smash()),
        combo: true,
        age: 5,
        wait_left: 6,
        frames: [Frame {
            idx: 1,
            loop_i: 2,
            loop_n: 3,
            param: 4,
            offset: 5,
        }]
        .into_iter()
        .collect(),
        parent_dir: dir(6),
        last_dir: dir(7),
        spot: Some(pt(8)),
    }
}

fn run() -> AttackRun {
    AttackRun {
        owner: EntityId(34),
        slot: 1,
        attack: AttackIndex(0),
        speed_pct: 110,
        width_pct: 120,
        phase: 0,
        tick: 2,
        facing: dir(3),
        aim: Aim::Entity(EntityId(4)),
        friendly: true,
        ranged: true,
        base: 5,
        atk_up_pct: 6,
        damage_pct: 130,
        type_bonus_pct: Some(Box::new([1; DamageType::ALL.len()])),
        age: 7,
        hits: vec![(EntityId(1), 8)],
        smash: Some(smash()),
        at: Some(pt(9)),
        throw: Some(pt(10)),
        homing: Some(EntityId(5)),
        spot: Some(pt(11)),
    }
}

fn expertise() -> ExpertiseGauge {
    ExpertiseGauge {
        level: 1,
        points: 2,
        next: 3,
        offered: 4,
        offer: Some(ExpertiseOffer {
            level: 5,
            kind: ExpertiseOfferKind::Style,
            candidates: vec![1, 2],
        }),
        taken: vec![1, 0],
        range_pct: 6,
        style: Some(1),
        path: vec![2],
    }
}

fn stars() -> StarField {
    StarField {
        misses: 1,
        on_map: vec![StarState {
            id: EntityId(40),
            at: pt(1),
            rarity: Rarity::Rare,
        }],
        due: vec![Due {
            kind: OfferKind::Boss,
            rarity: Rarity::Epic,
        }],
        offer: Some(OfferState {
            kind: OfferKind::Star,
            rarity: Rarity::Uncommon,
            candidates: vec![1, 2],
        }),
        rerolls_left: 2,
        banishes_left: 3,
        extra_candidates: 1,
        rarity: (Rarity::Common, Rarity::Legendary),
        rune_pool: 5,
        pool: vec![1, 2],
        taken: vec![1, 0],
        banished: vec![true, false],
        set_counts: vec![1, 0],
    }
}

fn progress() -> Progress {
    Progress {
        level: 2,
        xp: 3,
        stats: vec![StatRule {
            id: ContentId::from("stat"),
            effect: crate::run_stats::StatEffect::MaxHp,
            step: 1,
            limit: 2,
        }],
        priority: vec![0],
        pick_stats: true,
        offers_due: 4,
        offer_open: true,
        spent: vec![1],
        totals: [1; crate::run_stats::StatEffect::ALL.len()],
    }
}

fn player(p: &mut PlayerState) {
    p.facing = dir(2);
    p.target = Some(EntityId(50));
    p.evade_recovering = vec![3];
    p.combo = ComboState {
        path: 5,
        depth: 2,
        dash: true,
        buffered: Some(true),
        running: true,
        window_left: 1,
        dash_strike_left: 2,
        charge: Some(3),
        hold: Some(4),
        released: true,
        hand: true,
        auto_path: 3,
        auto_depth: 1,
        levels: 2,
        auto_levels: 3,
        charge_auto: true,
    };
    p.dash = Some(DashState {
        dir: dir(1),
        done: 2,
        left: 3,
    });
    p.skill = Some(SkillSlot {
        index: 1,
        level: 2,
        cooldown_left: 3,
    });
    p.pull = Some(PullState {
        at: pt(1),
        radius: 2,
        strength: 3,
        ticks_left: 4,
    });
    p.shield = Some(ShieldState {
        amount: 1,
        ticks_left: 2,
    });
    p.orbits = vec![OrbitState {
        reward: 1,
        orbs: 2,
        round: 3,
        cooldown_left: 4,
        angle: 5,
    }];
    p.runes = vec![HeldRune {
        rune: Rune::Thunder,
        cooldown_left: 6,
    }];
    p.statuses = statuses();
}

/// A state with every list non-empty and every `Option` `Some`, to perturb field by field.
pub(super) fn full_state(e: &Engine) -> State {
    let mut s = e.state.clone();
    player(&mut s.player);
    s.boss = Some(boss());
    s.companions = vec![agent()];
    s.enemies = vec![agent()];
    s.tasks = vec![task()];
    s.runs = vec![run()];
    s.ranged_cooling = vec![(5, RangedPool::Line)];
    s.orbs = vec![orb()];
    s.zones = vec![zone()];
    s.spawned = vec![1];
    s.progress = Some(progress());
    s.stars = Some(stars());
    s.heal_pickups = Some(vec![HealPickup {
        id: EntityId(60),
        at: pt(1),
    }]);
    s.expertise = Some(expertise());
    s.stardust = Some(7);
    s.echoes = vec![Echo {
        source: EntityId(1),
        target: EntityId(2),
        damage: 3,
        damage_type: Some(DamageType::Electro),
    }];
    let horde = s.horde.get_or_insert_with(|| Horde {
        boss_ticks: vec![1],
        no_call_ticks: vec![1],
        mid_boss_hp_pct: vec![1],
        spawned: 0,
        used: vec![false],
        held: vec![0],
    });
    horde.used = vec![true, false];
    horde.held = vec![1];
    s.move_index.lower(EntityId(1), 50);
    s
}

pub(super) fn engine() -> Engine {
    let mut v: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("time_limit")).unwrap();
    v["waves"] = serde_json::json!([]);
    let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let setup = RunSetup {
        schema_version: 1,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        pick_stats: false,
        difficulty: "normal".into(),
    };
    Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap()
}

/// What the invocations of `each_field_moves_the_hash!` share.
struct Probe {
    engine: Engine,
    base: State,
    base_hash: u64,
    failures: Vec<String>,
}

impl Probe {
    fn new() -> Probe {
        let mut engine = engine();
        assert!(
            engine.stage.attacks.len() >= 2,
            "the probe's stage has two attacks"
        );
        let base = full_state(&engine);
        engine.state = base.clone();
        let base_hash = engine.state_hash();
        Probe {
            engine,
            base,
            base_hash,
            failures: Vec::new(),
        }
    }

    /// Whether `change` to a copy of the base state moves the hash.
    fn moves(&mut self, change: impl FnOnce(&mut State)) -> bool {
        let mut s = self.base.clone();
        change(&mut s);
        self.engine.state = s;
        self.engine.state_hash() != self.base_hash
    }

    fn assert_clean(&self) {
        assert!(
            self.failures.is_empty(),
            "{} fields of the state are not covered by the hash:
{}",
            self.failures.len(),
            self.failures.join(
                "
"
            )
        );
    }
}

/// One state type: `get` finds a value of it in the base state. A field in `moves` must change
/// the hash when perturbed; one in `not_hashed` must not; one in `content` is a table or an
/// index that the content hash covers, which this test leaves alone. Every field of the type
/// is in exactly one list, or the invocation does not compile.
macro_rules! each_field_moves_the_hash {
    (
        $probe:ident, $ty:ident, $get:expr,
        moves: [$($m:ident),* $(,)?],
        not_hashed: [$($n:ident),* $(,)?],
        content: [$($c:ident),* $(,)?] $(,)?
    ) => {{
        #[allow(dead_code, unused_variables)]
        fn all_listed(t: &$ty) {
            let $ty { $($m: _,)* $($n: _,)* $($c: _,)* } = t;
        }
        let get: fn(&mut State) -> &mut $ty = $get;
        $(
            let moved = $probe.moves(|s| Perturb::perturb(&mut get(s).$m));
            if !moved {
                $probe.failures.push(format!(
                    "{}.{} does not move the hash",
                    stringify!($ty),
                    stringify!($m)
                ));
            }
        )*
        $(
            let moved = $probe.moves(|s| Perturb::perturb(&mut get(s).$n));
            if moved {
                $probe.failures.push(format!(
                    "{}.{} moves the hash but is listed as not hashed",
                    stringify!($ty),
                    stringify!($n)
                ));
            }
        )*
    }};
}

#[test]
fn every_state_field_moves_the_hash_or_is_listed_as_content() {
    let mut p = Probe::new();

    each_field_moves_the_hash!(p, State, |s| s,
        moves: [tick, rng, outcome, player, boss, companions, enemies, tasks, runs, ranged_cooling, orbs, zones, spawned, next_id, next_orb_group, progress, stars, heal_pickups, expertise, max_attack_speed_pct, enemy_attack_damage, max_rune_haste_pct, horde, stardust, regen, hp_pct, damage_pct, crit_seed, echoes],
        not_hashed: [move_index],
        content: [scratch, kinds, match_ups, runes, stun]);
    each_field_moves_the_hash!(p, Streams, |s| &mut s.rng,
        moves: [spawn, attack, loot, offer, horde],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Echo, |s| &mut s.echoes[0],
        moves: [source, target, damage, damage_type],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Horde, |s| s.horde.as_mut().unwrap(),
        moves: [spawned, used, held],
        not_hashed: [],
        content: [boss_ticks, no_call_ticks, mid_boss_hp_pct]);
    each_field_moves_the_hash!(p, ExpertiseGauge, |s| s.expertise.as_mut().unwrap(),
        moves: [level, points, next, offered, offer, taken, range_pct, style, path],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, ExpertiseOffer, |s| s.expertise.as_mut().unwrap().offer.as_mut().unwrap(),
        moves: [level, kind, candidates],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, StarField, |s| s.stars.as_mut().unwrap(),
        moves: [misses, on_map, due, offer, rerolls_left, banishes_left, extra_candidates, rarity, rune_pool, pool, taken, banished, set_counts],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, StarState, |s| &mut s.stars.as_mut().unwrap().on_map[0],
        moves: [id, at, rarity],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Due, |s| &mut s.stars.as_mut().unwrap().due[0],
        moves: [kind, rarity],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, OfferState, |s| s.stars.as_mut().unwrap().offer.as_mut().unwrap(),
        moves: [kind, rarity, candidates],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, HealPickup, |s| &mut s.heal_pickups.as_mut().unwrap()[0],
        moves: [id, at],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Progress, |s| s.progress.as_mut().unwrap(),
        moves: [level, xp, priority, pick_stats, offers_due, offer_open, spent, totals],
        not_hashed: [stats],
        content: []);
    each_field_moves_the_hash!(p, PlayerState, |s| &mut s.player,
        moves: [at, hp, max_hp, defence, invulnerable, facing, target, evade_charges, evade_recovering, held, combo, dash, skill, pull, shield, orbits, range_pct, rune_haste_pct, pickup_radius_pct, skill_cooldown_pct, kill_heal, kill_heal_every, kills_counted, crit_chance_pct, crit_damage_pct, echo_hit_pct, burst_pct, burst_radius, burst_depth, burst_depth_extra, type_bonus_pct, runes, aggro, statuses],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, ComboState, |s| &mut s.player.combo,
        moves: [path, depth, dash, buffered, running, window_left, dash_strike_left, charge, hold, released, hand, auto_path, auto_depth, levels, auto_levels, charge_auto],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, SkillSlot, |s| s.player.skill.as_mut().unwrap(),
        moves: [index, level, cooldown_left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, OrbitState, |s| &mut s.player.orbits[0],
        moves: [reward, orbs, round, cooldown_left, angle],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, HeldRune, |s| &mut s.player.runes[0],
        moves: [rune, cooldown_left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, PullState, |s| s.player.pull.as_mut().unwrap(),
        moves: [at, radius, strength, ticks_left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, ShieldState, |s| s.player.shield.as_mut().unwrap(),
        moves: [amount, ticks_left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, DashState, |s| s.player.dash.as_mut().unwrap(),
        moves: [dir, done, left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, BossState, |s| s.boss.as_mut().unwrap(),
        moves: [id, origin, no_call, at, hp, max_hp, defence, statuses, phase, phase_age, invulnerable, step, wait_left, order, rules, parts, elites_called, age],
        not_hashed: [],
        content: [boss, kind]);
    each_field_moves_the_hash!(p, PartState, |s| &mut s.boss.as_mut().unwrap().parts[0],
        moves: [id, hp, max_hp, defence, offset, radius, statuses],
        not_hashed: [],
        content: [name, kind]);
    each_field_moves_the_hash!(p, AgentKey, |s| &mut s.enemies[0].key,
        moves: [source, index, rule],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, AgentState, |s| &mut s.enemies[0],
        moves: [id, key, at, hp, max_hp, defence, age, rules, statuses, aggro, order, orbit_angle, crowded, burst, stun_immune, elite, leave, shrine],
        not_hashed: [],
        content: [def, kind]);
    each_field_moves_the_hash!(p, ShrineState, |s| s.enemies[0].shrine.as_deref_mut().unwrap(),
        moves: [fight, guardians],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, LeaveState, |s| s.enemies[0].leave.as_mut().unwrap(),
        moves: [disturbed, go],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, EliteState, |s| s.enemies[0].elite.as_mut().unwrap(),
        moves: [home, called, disguised],
        not_hashed: [boss_call],
        content: []);
    each_field_moves_the_hash!(p, MoveOrder, |s| s.enemies[0].order.as_mut().unwrap(),
        moves: [to, ticks_left],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, RuleState, |s| &mut s.enemies[0].rules[0],
        moves: [cooldown_left, charges_left, fired],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, StatusState, |s| &mut s.player.statuses[0],
        moves: [kind, source, remaining, total, stacks, value_pct, dot],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, DotState, |s| s.player.statuses[0].dot.as_mut().unwrap(),
        moves: [damage, damage_type, atk_up_pct, type_bonus_pct, interval, next_in],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, AttackTask, |s| &mut s.tasks[0],
        moves: [owner, base, origin, aim, friendly, damage, damage_type, breaks_charge, smash, combo, age, wait_left, frames, parent_dir, last_dir, spot],
        not_hashed: [program],
        content: []);
    each_field_moves_the_hash!(p, AttackRun, |s| &mut s.runs[0],
        moves: [owner, slot, speed_pct, width_pct, phase, tick, facing, aim, friendly, ranged, base, atk_up_pct, damage_pct, type_bonus_pct, age, hits, smash, at, throw, homing, spot],
        not_hashed: [attack],
        content: []);
    each_field_moves_the_hash!(p, Frame, |s| &mut s.tasks[0].frames[0],
        moves: [idx, loop_i, loop_n, param, offset],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, PathStep, |s| &mut s.tasks[0].base[0],
        moves: [node, then],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Pull, |s| match s.orbs[0].smash.as_mut().unwrap() { Smash::Pull(p) => p, _ => unreachable!() },
        moves: [distance, ticks],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, OrbState, |s| &mut s.orbs[0],
        moves: [at, vx, vy, dir, style, owner, damage, damage_type, breaks_charge, smash, combo, hook, breakable, life, stop, link],
        not_hashed: [program],
        content: []);
    each_field_moves_the_hash!(p, OrbLink, |s| s.orbs[0].link.as_mut().unwrap(),
        moves: [group, slot, of, width, damage, rehit_ticks, last_hit],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Hook, |s| s.orbs[0].hook.as_deref_mut().unwrap(),
        moves: [path, left, owner, aim, damage, damage_type, breaks_charge, smash, combo],
        not_hashed: [program],
        content: []);
    each_field_moves_the_hash!(p, ZoneState, |s| &mut s.zones[0],
        moves: [owner, friendly, shape, at, dir, wind_left, active_left, damage, damage_type, breaks_charge, warn_only, hits],
        not_hashed: [wind_ticks, active_ticks, program],
        content: []);
    each_field_moves_the_hash!(p, Dir, |s| &mut s.player.facing,
        moves: [x, y],
        not_hashed: [],
        content: []);
    each_field_moves_the_hash!(p, Point, |s| &mut s.player.at,
        moves: [x, y],
        not_hashed: [],
        content: []);
    p.assert_clean();
}
