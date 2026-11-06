//! Tests for the equipment slots (ADR-SIM-3 point 2, ADR-SIM-7): the layout at tick 0, the
//! build from the equipped set, the grants and the equip and replace rules.

use super::equipment::{Held, SlotKind};
use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;
use crate::rune::Rune;

fn setup(rune: Option<&str>, lumine: Option<&str>) -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        starting_rune: rune.map(|r| r.into()),
        lumine: u8::from(lumine.is_some()),
        lumine_star: lumine.map(|l| l.into()),
        ..RunSetup::default()
    }
}

fn state_of(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// The layout the slots take until the pack carries it: Attack 5, Defence 2, Mobility 2, Rune 3.
fn layout() -> Vec<SlotKind> {
    let mut v = Vec::new();
    for (kind, n) in [
        (SlotKind::Category(0), 5),
        (SlotKind::Category(1), 2),
        (SlotKind::Category(2), 2),
        (SlotKind::Rune, 3),
    ] {
        v.extend(std::iter::repeat_n(kind, n));
    }
    v
}

/// T1: an equipment run starts with the layout, all empty; an offers run has none.
#[test]
fn an_equipment_run_starts_with_the_layout() {
    let e = Engine::start(&setup(None, None)).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    let kinds: Vec<SlotKind> = eq.slots.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, layout());
    assert!(
        eq.slots
            .iter()
            .all(|s| s.held.is_none() && s.level == 0 && s.expertise == 0)
    );
    assert!(eq.lumine.is_none());

    let e = Engine::start(&setup(None, Some("max_hp_common"))).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    assert_eq!(eq.slots.len(), 12);
    assert!(eq.lumine.is_some());

    let e = Engine::start(&RunSetup::default()).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    assert!(eq.slots.is_empty() && eq.lumine.is_none());
}

/// T2: the starting rune fills rune slot 0 and the runes the Main fires, not the inventory.
#[test]
fn the_starting_rune_fills_rune_slot_0() {
    let plain = Engine::start(&setup(None, None)).expect("starts");
    let e = Engine::start(&setup(Some("thunder"), None)).expect("starts");
    let (s, p) = (state_of(&e), state_of(&plain));
    let c = s.players.career(PlayerIdx::SOLO);
    assert_eq!(c.equipment.slots[9].held, Some(Held::Rune(Rune::Thunder)));
    assert!(c.equipment.slots[10].held.is_none() && c.equipment.slots[11].held.is_none());
    assert!(c.equipment.inventory.is_empty());
    let runes: Vec<Rune> = s.players[PlayerIdx::SOLO]
        .runes
        .iter()
        .map(|h| h.rune)
        .collect();
    assert_eq!(runes, [Rune::Thunder]);
    let q = p.players.career(PlayerIdx::SOLO);
    assert!(q.equipment.slots.iter().all(|s| s.held.is_none()));
    assert!(p.players[PlayerIdx::SOLO].runes.is_empty());
    assert_eq!(c.offer.state(), q.offer.state(), "no draw moved");
}

/// T3: the Lumine star is in the build at tick 0 (its +20,000 max hp, hp full).
#[test]
fn the_lumine_star_is_in_the_build_at_tick_0() {
    let plain = Engine::start(&setup(None, None)).expect("starts");
    let e = Engine::start(&setup(None, Some("max_hp_common"))).expect("starts");
    let (s, p) = (state_of(&e), state_of(&plain));
    let (me, was) = (&s.players[PlayerIdx::SOLO], &p.players[PlayerIdx::SOLO]);
    assert_eq!(me.max_hp, was.max_hp + 20_000);
    assert_eq!(me.hp, me.max_hp);
    let total = |s: &State| {
        s.players
            .career(PlayerIdx::SOLO)
            .progress
            .as_ref()
            .expect("levels")
            .totals[StatEffect::MaxHp as usize]
    };
    assert_eq!(total(s), total(p) + 20_000);
}

// --- the grants follow the equipped set (ADR-SIM-7) ---------------------------------------------

mod grants {
    use super::super::equipment::{Equipment, Held};
    use super::super::state::{OrbitState, PlayerIdx, SkillSlot, State};
    use super::super::{RunDifficulty, RunPool, World, initial_state, slots};
    use crate::rewards::{Rarity, Rewards};
    use crate::run_content::RunContent;
    use crate::run_stats::StatEffect;
    use crate::version::Versions;
    use serde_json::{Value, json};

    const NOVA: u16 = 1;
    const WAR_CRY: u16 = 2;
    const NOVA2: u16 = 3;
    const ORBIT_A: u16 = 4;
    const ORBIT_B: u16 = 5;
    const EVADE_A: u16 = 6;
    const EVADE_B: u16 = 7;
    const EVADE_C: u16 = 8;
    const REROLL: u16 = 9;
    const BUNDLE: u16 = 10;
    pub(super) const GROWTH: u16 = 11;

    pub(super) fn star(id: &str, rarity: &str, effects: Vec<Value>) -> Value {
        json!({ "id": id, "rarity": rarity, "limit": 3, "effects": effects })
    }

    fn orbit(orbs: u32, cooldown: u32) -> Value {
        json!({ "effect": "orbit", "amount": orbs, "orbit": { "attack": { "inline": [
            { "kind": "active", "ticks": 20, "type": "slash", "damage": 1500,
              "hit_box": { "type": "circle", "offset": { "forward": 15360 }, "radius": 3072 } }
        ] }, "turn_per_tick": 1280, "cooldown_ticks": cooldown } })
    }

    fn skill(skill: &str, levels: u32) -> Value {
        json!({ "effect": "stage_skill", "skill": skill, "amount": levels })
    }

    pub(super) fn rarities() -> Vec<Value> {
        [
            "common",
            "uncommon",
            "rare",
            "epic",
            "legendary",
            "supernova",
        ]
        .iter()
        .enumerate()
        .map(|(i, r)| json!({ "rarity": r, "lumine_cost": i + 1 }))
        .collect()
    }

    fn rewards() -> Rewards {
        let rarities = rarities();
        let list = vec![
            star(
                "hp",
                "common",
                vec![json!({ "effect": "max_hp", "amount": 100 })],
            ),
            star("nova", "common", vec![skill("nova", 1)]),
            star("war_cry", "common", vec![skill("war_cry", 1)]),
            star("nova2", "common", vec![skill("nova", 2)]),
            star("orbit_a", "common", vec![orbit(2, 120)]),
            star("orbit_b", "common", vec![orbit(3, 90)]),
            star(
                "evade_a",
                "common",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "evade_b",
                "common",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "evade_c",
                "common",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "reroll",
                "common",
                vec![
                    json!({ "effect": "rerolls", "amount": 2 }),
                    json!({ "effect": "banishes", "amount": 1 }),
                ],
            ),
            star(
                "bundle",
                "common",
                vec![
                    json!({ "effect": "random_rune" }),
                    json!({ "effect": "attack_pct", "amount": 3 }),
                ],
            ),
            star(
                "growth",
                "common",
                vec![
                    json!({ "effect": "rerolls", "amount": 2 }),
                    json!({ "effect": "banishes", "amount": 1 }),
                    json!({ "effect": "offer_candidates", "amount": 1 }),
                    json!({ "effect": "xp_pct", "amount": 10 }),
                ],
            ),
        ];
        let mut file = json!({ "rarities": rarities, "rewards": list });
        crate::sim::starlight::with_equipment_rules(&mut file);
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("validates")
    }

    pub(super) struct Fixture {
        pub stage: World,
        pub rewards: Rewards,
        pub s: State,
    }

    fn fixture() -> Fixture {
        with(rewards())
    }

    /// A solo state over `rewards`, with the slot layout and room for every star in the pool.
    pub(super) fn with(rewards: Rewards) -> Fixture {
        build(rewards, true)
    }

    /// As `with`, over a stage with no `levels`: the player has no stat totals.
    fn without_levels(rewards: Rewards) -> Fixture {
        build(rewards, false)
    }

    fn build(rewards: Rewards, levels: bool) -> Fixture {
        let mut stage = RunContent::bundled().trial().clone();
        if !levels {
            stage.levels = None;
        }
        stage.player.evade.charges = 1;
        stage.player.evade.max_charges = Some(3);
        let mut stage = World::for_test(stage);
        // The slots are equipment mode's: the Growth effects are stats there.
        stage.equipment_mode = true;
        stage.equipment_rules = slots::bundled_rules().clone();
        let pool = RunPool {
            rewards: (0..u16::try_from(rewards.rewards().len()).unwrap()).collect(),
            extra_rerolls: 0,
            difficulty: RunDifficulty {
                hp_pct: 100,
                damage_pct: 100,
                rarity: (Rarity::Common, Rarity::Supernova),
            },
        };
        let mut s = initial_state(
            &stage,
            RunContent::bundled(),
            pool,
            1,
            crate::kinds::KindTable::of_stage(&stage).unwrap(),
        );
        s.players.career_mut(PlayerIdx::SOLO).equipment.slots =
            slots::fresh_slots(slots::bundled_rules());
        Fixture { stage, rewards, s }
    }

    impl Fixture {
        /// Puts `place` (or nothing) into slot `at` and writes the change.
        pub(super) fn set(&mut self, at: usize, place: Option<u16>) {
            self.change(|e| {
                e.slots[at].held = place.map(Held::Star);
                e.slots[at].level = u8::from(place.is_some());
            });
        }

        pub(super) fn change(&mut self, f: impl FnOnce(&mut Equipment)) {
            let (player, career) = self.s.players.both_mut(PlayerIdx::SOLO);
            let before = slots::equipped_levels(&career.equipment);
            f(&mut career.equipment);
            slots::refresh(&self.stage, &self.rewards, player, career, &before);
        }

        fn skill(&self) -> Option<SkillSlot> {
            self.s.players[PlayerIdx::SOLO].skill
        }

        fn orbits(&self) -> Vec<OrbitState> {
            self.s.players[PlayerIdx::SOLO].orbits.clone()
        }

        fn charges(&self) -> u8 {
            self.s.players[PlayerIdx::SOLO].evade_charges
        }
    }

    fn nova_index(f: &Fixture) -> u16 {
        u16::try_from(f.rewards.stage_skills().index_of(&"nova".into()).unwrap()).unwrap()
    }

    /// T7: the skill is the first equipped skill star in slot order.
    #[test]
    fn the_skill_follows_the_equipped_set() {
        let mut f = fixture();
        assert_eq!(f.skill(), None);
        f.set(0, Some(NOVA));
        let nova = nova_index(&f);
        assert_eq!(
            f.skill(),
            Some(SkillSlot {
                index: nova,
                level: 1,
                cooldown_left: 0
            })
        );
        // A second skill star in a later slot changes nothing: the first in slot order wins.
        f.set(1, Some(WAR_CRY));
        assert_eq!(f.skill().map(|s| s.index), Some(nova));
        // The same skill at another grade keeps the cooldown and takes the new level.
        f.s.players[PlayerIdx::SOLO]
            .skill
            .as_mut()
            .unwrap()
            .cooldown_left = 40;
        f.set(0, Some(NOVA2));
        assert_eq!(
            f.skill(),
            Some(SkillSlot {
                index: nova,
                level: 2,
                cooldown_left: 40
            })
        );
        // Replacing the first out: the second's skill arrives ready.
        f.set(0, None);
        let war_cry = u16::try_from(
            f.rewards
                .stage_skills()
                .index_of(&"war_cry".into())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            f.skill(),
            Some(SkillSlot {
                index: war_cry,
                level: 1,
                cooldown_left: 0
            })
        );
        f.set(1, None);
        assert_eq!(f.skill(), None);
    }

    /// T8: the orbits are the equipped orbit stars in slot order, each keeping its run state.
    #[test]
    fn orbits_follow_the_equipped_set() {
        let mut f = fixture();
        f.set(0, Some(ORBIT_A));
        assert_eq!(f.orbits().len(), 1);
        assert_eq!((f.orbits()[0].reward, f.orbits()[0].orbs), (ORBIT_A, 2));
        {
            let o = &mut f.s.players[PlayerIdx::SOLO].orbits[0];
            (o.round, o.cooldown_left, o.angle) = (2, 77, 5120);
        }
        f.set(1, Some(ORBIT_B));
        let o = f.orbits();
        assert_eq!(o.len(), 2);
        assert_eq!(
            (o[0].reward, o[0].round, o[0].cooldown_left, o[0].angle),
            (ORBIT_A, 2, 77, 5120),
            "the first keeps its run state"
        );
        assert_eq!(
            (
                o[1].reward,
                o[1].orbs,
                o[1].round,
                o[1].cooldown_left,
                o[1].angle
            ),
            (ORBIT_B, 3, 0, 0, 0),
            "the new one arrives ready"
        );
        // The first replaced out: removed, the other keeps its state.
        {
            let o = &mut f.s.players[PlayerIdx::SOLO].orbits[1];
            (o.round, o.cooldown_left, o.angle) = (3, 9, 256);
        }
        f.set(0, None);
        let o = f.orbits();
        assert_eq!(o.len(), 1);
        assert_eq!(
            (o[0].reward, o[0].round, o[0].cooldown_left, o[0].angle),
            (ORBIT_B, 3, 9, 256)
        );
    }

    /// T9: the evade charges follow the equipped set, from the stage's start and up to its most.
    #[test]
    fn evade_charges_follow_the_equipped_set() {
        let mut f = fixture();
        assert_eq!(f.charges(), 1);
        f.set(0, Some(EVADE_A));
        assert_eq!(f.charges(), 2);
        // Spend one, then replace the star out: back to the stage's own, never below it.
        f.s.players[PlayerIdx::SOLO].evade_charges = 1;
        f.set(0, None);
        assert_eq!(f.charges(), 1);
        // Two stars and a third: the cap is 3.
        f.set(0, Some(EVADE_A));
        f.set(1, Some(EVADE_B));
        assert_eq!(f.charges(), 3);
        f.set(2, Some(EVADE_C));
        assert_eq!(f.charges(), 3);
        // The cap held, so taking one out leaves the charges the others still give.
        f.set(2, None);
        assert_eq!(f.charges(), 3);
        f.set(1, None);
        assert_eq!(f.charges(), 2);
    }

    /// The reroll star's offers in equipment mode: rerolls and banishes are stats, the offers
    /// stay as they were. A random rune in a bundle draws nothing.
    #[test]
    fn rerolls_banishes_and_the_random_rune_grant_nothing_else() {
        let mut f = fixture();
        let o = |f: &Fixture| {
            let o = f.s.players.career(PlayerIdx::SOLO).offers.as_ref().unwrap();
            (o.rerolls_left, o.banishes_left, o.extra_candidates)
        };
        let base = o(&f);
        f.set(0, Some(REROLL));
        assert_eq!(o(&f), base, "equipment mode writes stats, not offers");
        assert_eq!(f.total(StatEffect::Rerolls), 2);
        assert_eq!(f.total(StatEffect::Banishes), 1);
        f.set(0, None);
        assert_eq!(o(&f), base);
        assert_eq!(f.total(StatEffect::Rerolls), 0);
        assert_eq!(f.total(StatEffect::Banishes), 0);

        let career = f.s.players.career(PlayerIdx::SOLO);
        let (offer, runes) = (
            career.offer.state(),
            f.s.players[PlayerIdx::SOLO].runes.clone(),
        );
        f.set(0, Some(BUNDLE));
        let career = f.s.players.career(PlayerIdx::SOLO);
        assert_eq!(career.offer.state(), offer, "no draw");
        assert_eq!(f.s.players[PlayerIdx::SOLO].runes, runes, "no rune");
    }

    /// T7: a Growth star writes its amounts to the stat totals in equipment mode and a replace
    /// takes them back exactly; the offers stay as they were.
    #[test]
    fn a_growth_star_writes_stats_in_equipment_mode() {
        let mut f = fixture();
        let offers = |f: &Fixture| {
            let o = f.s.players.career(PlayerIdx::SOLO).offers.as_ref().unwrap();
            (o.rerolls_left, o.banishes_left, o.extra_candidates)
        };
        let base = offers(&f);
        f.set(0, Some(GROWTH));
        assert_eq!(f.total(StatEffect::Rerolls), 2);
        assert_eq!(f.total(StatEffect::Banishes), 1);
        assert_eq!(f.total(StatEffect::OfferCandidates), 1);
        assert_eq!(offers(&f), base);
        f.set(0, None);
        for stat in [
            StatEffect::Rerolls,
            StatEffect::Banishes,
            StatEffect::OfferCandidates,
        ] {
            assert_eq!(f.total(stat), 0, "{stat:?}");
        }
        assert_eq!(offers(&f), base);
    }

    /// T8: the Growth amount is not level-scaled, `xp_pct` is.
    #[test]
    fn the_growth_amount_is_not_level_scaled() {
        let mut f = fixture();
        f.set(0, Some(GROWTH));
        let xp1 = f.total(StatEffect::XpPct);
        f.change(|e| e.slots[0].level = 10);
        assert_eq!(f.total(StatEffect::Rerolls), 2);
        assert_eq!(f.total(StatEffect::Banishes), 1);
        assert_eq!(f.total(StatEffect::OfferCandidates), 1);
        assert!(
            f.total(StatEffect::XpPct) > xp1,
            "xp_pct scales with the level"
        );
    }

    /// T10: a stage with no `levels` has no stat totals: the Growth stats are dropped, with no
    /// panic and nothing else moved.
    #[test]
    fn a_stage_without_levels_drops_the_growth_stats() {
        let mut f = without_levels(rewards());
        assert!(f.s.players.career(PlayerIdx::SOLO).progress.is_none());
        let before = f.s.players.career(PlayerIdx::SOLO).offers.clone();
        f.set(0, Some(GROWTH));
        assert!(f.s.players.career(PlayerIdx::SOLO).progress.is_none());
        assert_eq!(f.s.players.career(PlayerIdx::SOLO).offers, before);
    }
}

// --- equip, replace, switch and the rune slots (ADR-SIM-7) -------------------------------------

mod rules {
    use super::super::build::Build;
    use super::super::equipment::Held;
    use super::super::slots::{self, SlotError};
    use super::super::state::{HeldRune, PlayerIdx};
    use super::grants::{Fixture, rarities, star, with};
    use crate::constellations::Constellations;
    use crate::rewards::Rewards;
    use crate::run_stats::StatEffect;
    use crate::rune::Rune;
    use crate::version::Versions;
    use serde_json::{Value, json};

    const HP_C: u16 = 0;
    const HP_R: u16 = 1;
    const ATK: u16 = 2;
    const ATK_HP: u16 = 3;
    const SPEED: u16 = 4;
    const XP: u16 = 5;
    const RUNE: u16 = 6;
    const CRIT: u16 = 7;
    const REGEN: u16 = 8;

    // The slot indices of the layout.
    const ATTACK0: u32 = 0;
    const ATTACK1: u32 = 1;
    const DEFENCE0: u32 = 5;
    const DEFENCE1: u32 = 6;
    const MOBILITY0: u32 = 7;
    const RUNE0: u32 = 9;
    const RUNE1: u32 = 10;
    const RUNE2: u32 = 11;
    const LUMINE: u32 = 12;

    fn fx(effect: &str, amount: u32) -> Value {
        json!({ "effect": effect, "amount": amount })
    }

    /// A named star (epic, taken once), as a set member must be.
    fn named(id: &str, name: &str, effect: Value) -> Value {
        json!({ "id": id, "rarity": "epic", "named": true, "star": name, "limit": 1,
            "effects": [effect] })
    }

    /// The file: two max hp stars (one option, two grades), attack, a star of two effects, speed,
    /// xp (Growth), the lone random rune, crit and regen, with the set `duo` of speed and crit
    /// whose tier 1 (both members) gives 7 attack.
    fn rewards() -> Rewards {
        let list = vec![
            star("hp_c", "common", vec![fx("max_hp", 100)]),
            star("hp_r", "rare", vec![fx("max_hp", 250)]),
            star("atk", "common", vec![fx("attack_pct", 5)]),
            star(
                "atk_hp",
                "rare",
                vec![fx("attack_pct", 4), fx("max_hp", 50)],
            ),
            named("speed", "Swift", fx("speed_pct", 10)),
            star("xp", "common", vec![fx("xp_pct", 10)]),
            json!({ "id": "random_rune", "rarity": "common", "effects": [{ "effect": "random_rune" }] }),
            named("crit", "Keen", fx("crit_chance_pct", 5)),
            star("regen", "common", vec![fx("regen", 2)]),
        ];
        let mut file = json!({ "rarities": rarities(), "rewards": list });
        crate::sim::starlight::with_equipment_rules(&mut file);
        let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
            .expect("the file validates");
        let sets = json!({ "constellations": [{
            "id": "duo", "name": "The duo", "members": ["speed", "crit"],
            "tiers": [{ "threshold": 2, "effects": [fx("attack_pct", 7)] }] }] });
        let sets =
            Constellations::parse_with(sets.to_string().as_bytes(), Versions::CURRENT, &rewards)
                .expect("the duo validates");
        rewards.with_constellations(sets)
    }

    fn fixture() -> Fixture {
        with(rewards())
    }

    impl Fixture {
        fn put(&mut self, items: &[Held]) {
            self.s
                .players
                .career_mut(PlayerIdx::SOLO)
                .equipment
                .inventory
                .extend_from_slice(items);
        }

        fn eq(&self) -> &super::super::equipment::Equipment {
            &self.s.players.career(PlayerIdx::SOLO).equipment
        }

        fn equip(&mut self, item: u32, slot: u32) -> Result<(), SlotError> {
            let (player, career) = self.s.players.both_mut(PlayerIdx::SOLO);
            slots::equip(&self.stage, &self.rewards, player, career, item, slot)
        }

        fn replace(&mut self, item: u32, slot: u32) -> Result<(), SlotError> {
            let (player, career) = self.s.players.both_mut(PlayerIdx::SOLO);
            slots::replace(&self.stage, &self.rewards, player, career, item, slot)
        }

        pub(super) fn total(&self, stat: StatEffect) -> u32 {
            self.s
                .players
                .career(PlayerIdx::SOLO)
                .progress
                .as_ref()
                .unwrap()
                .totals[stat as usize]
        }

        fn set_counts(&self) -> Vec<u8> {
            self.s
                .players
                .career(PlayerIdx::SOLO)
                .offers
                .as_ref()
                .unwrap()
                .set_counts[..1]
                .to_vec()
        }

        /// Gives slot `at` the Lumine star `place`.
        fn lumine(&mut self, place: u16) {
            self.change(|e| {
                e.lumine = Some(super::super::equipment::EquipSlot {
                    kind: super::super::equipment::SlotKind::Lumine,
                    held: Some(Held::Star(place)),
                    level: 1,
                    expertise: 0,
                });
            });
        }
    }

    fn star_at(f: &Fixture, slot: usize) -> Option<Held> {
        f.eq().slots[slot].held
    }

    /// T11: an equip moves the star from the inventory into the slot and the build follows.
    #[test]
    fn equip_puts_an_inventory_star_into_an_empty_slot() {
        let mut f = fixture();
        f.put(&[Held::Star(ATK), Held::Star(HP_C)]);
        let atk = f.total(StatEffect::AttackPct);
        assert_eq!(f.equip(0, ATTACK0), Ok(()));
        assert_eq!(f.eq().inventory, [Held::Star(HP_C)]);
        assert_eq!(star_at(&f, 0), Some(Held::Star(ATK)));
        assert_eq!((f.eq().slots[0].level, f.eq().slots[0].expertise), (1, 0));
        assert_eq!(f.total(StatEffect::AttackPct), atk + 5);
        // The build is the build of the equipped counts.
        let counts = slots::equipped_counts(&f.rewards, f.eq());
        let build = Build::of(&f.stage, &f.rewards, &counts);
        assert_eq!(build.stats[StatEffect::AttackPct as usize], 5);
        assert_eq!(f.set_counts(), [0]);
    }

    /// T12: a replace puts the displaced star at the item's place; a star sharing no option halves
    /// the level (Lv3 to Lv1) and resets the kills.
    #[test]
    fn replace_returns_the_displaced_star_to_the_item_s_place() {
        let mut f = fixture();
        f.set(0, Some(ATK));
        f.change(|e| {
            e.slots[0].level = 3;
            e.slots[0].expertise = 17;
        });
        f.put(&[Held::Star(SPEED), Held::Star(CRIT), Held::Star(HP_C)]);
        let atk = f.total(StatEffect::AttackPct);
        assert_eq!(f.replace(1, ATTACK0), Ok(()));
        assert_eq!(
            f.eq().inventory,
            [Held::Star(SPEED), Held::Star(ATK), Held::Star(HP_C)]
        );
        assert_eq!(star_at(&f, 0), Some(Held::Star(CRIT)));
        assert_eq!((f.eq().slots[0].level, f.eq().slots[0].expertise), (1, 0));
        assert_eq!(f.total(StatEffect::AttackPct), atk - 5);
    }

    /// T13: a same-option star of another grade switches in its slot.
    #[test]
    fn a_same_option_star_of_another_grade_switches_in_its_slot() {
        let mut f = fixture();
        f.set(5, Some(HP_C));
        f.put(&[Held::Star(HP_R), Held::Star(REGEN)]);
        assert!(slots::is_switch(&f.rewards, f.eq(), 0, DEFENCE0));
        assert!(!slots::is_switch(&f.rewards, f.eq(), 1, DEFENCE0));
        assert_eq!(f.replace(0, DEFENCE0), Ok(()));
        assert_eq!(star_at(&f, 5), Some(Held::Star(HP_R)));
        assert_eq!(f.eq().slots[5].level, 1);
        assert_eq!(f.eq().inventory[0], Held::Star(HP_C));
    }

    /// T14: a star sharing an option with one equipped in another slot is refused.
    #[test]
    fn a_same_option_star_in_another_slot_is_refused() {
        let mut f = fixture();
        f.set(5, Some(HP_C));
        f.put(&[Held::Star(HP_R), Held::Star(ATK), Held::Star(ATK_HP)]);
        assert_eq!(
            f.equip(0, DEFENCE1),
            Err(SlotError::OptionHeld { slot: DEFENCE0 })
        );
        // The kind of slot is checked before the option.
        assert_eq!(
            f.equip(0, ATTACK0),
            Err(SlotError::WrongSlot { slot: ATTACK0 })
        );
        // A star of two effects sharing one entry is refused too.
        assert_eq!(f.equip(1, ATTACK0), Ok(()));
        assert_eq!(f.eq().inventory.len(), 2);
        assert_eq!(
            f.equip(1, ATTACK1),
            Err(SlotError::OptionHeld { slot: ATTACK0 })
        );
        // And so is a star sharing with the Lumine star.
        let mut f = fixture();
        f.lumine(HP_C);
        f.put(&[Held::Star(HP_R)]);
        assert_eq!(
            f.equip(0, DEFENCE0),
            Err(SlotError::OptionHeld { slot: LUMINE })
        );
        // Replacing the Lumine star by a same-option star is a switch.
        assert!(slots::is_switch(&f.rewards, f.eq(), 0, LUMINE));
        assert_eq!(f.replace(0, LUMINE), Ok(()));
        assert_eq!(f.eq().lumine.as_ref().unwrap().held, Some(Held::Star(HP_R)));
    }

    /// T15: the sets count the stars equipped, the Lumine star included, never the inventory.
    #[test]
    fn sets_count_equipped_stars_only() {
        let mut f = fixture();
        f.put(&[Held::Star(SPEED), Held::Star(CRIT), Held::Star(ATK)]);
        assert_eq!(f.set_counts(), [0]);
        let atk = f.total(StatEffect::AttackPct);
        assert_eq!(f.equip(0, MOBILITY0), Ok(()));
        assert_eq!(f.set_counts(), [1]);
        assert_eq!(
            f.total(StatEffect::AttackPct),
            atk,
            "no tier with one member"
        );
        assert_eq!(f.equip(0, ATTACK0), Ok(()));
        assert_eq!(f.set_counts(), [2]);
        assert_eq!(f.total(StatEffect::AttackPct), atk + 7, "the tier arrives");
        // Replacing a member out: the tier falls at once.
        assert_eq!(f.replace(0, ATTACK0), Ok(()));
        assert_eq!(f.set_counts(), [1]);
        assert_eq!(
            f.total(StatEffect::AttackPct),
            atk + 5,
            "only the atk star is left"
        );

        // The Lumine member counts as one equipped.
        let mut f = fixture();
        let base = f.total(StatEffect::AttackPct);
        f.lumine(SPEED);
        assert_eq!(f.set_counts(), [1]);
        f.put(&[Held::Star(CRIT)]);
        assert_eq!(f.equip(0, ATTACK0), Ok(()));
        assert_eq!(f.set_counts(), [2]);
        assert_eq!(f.total(StatEffect::AttackPct), base + 7);
    }

    /// T16: a star goes only to its category; the Lumine slot takes any star and no rune.
    #[test]
    fn a_star_goes_only_to_its_category() {
        let mut f = fixture();
        f.lumine(HP_C);
        f.put(&[
            Held::Star(ATK),
            Held::Star(XP),
            Held::Star(RUNE),
            Held::Rune(Rune::Bleed),
        ]);
        assert_eq!(
            f.equip(0, DEFENCE0),
            Err(SlotError::WrongSlot { slot: DEFENCE0 })
        );
        assert_eq!(
            f.equip(1, ATTACK0),
            Err(SlotError::WrongSlot { slot: ATTACK0 })
        );
        assert_eq!(
            f.equip(1, MOBILITY0),
            Err(SlotError::WrongSlot { slot: MOBILITY0 })
        );
        assert_eq!(f.equip(0, RUNE0), Err(SlotError::WrongSlot { slot: RUNE0 }));
        assert_eq!(
            f.equip(3, ATTACK0),
            Err(SlotError::WrongSlot { slot: ATTACK0 })
        );
        for slot in [ATTACK0, DEFENCE0] {
            assert_eq!(f.equip(2, slot), Err(SlotError::NotEquippable { item: 2 }));
        }
        assert_eq!(
            f.replace(2, LUMINE),
            Err(SlotError::NotEquippable { item: 2 })
        );
        assert_eq!(
            f.replace(3, LUMINE),
            Err(SlotError::WrongSlot { slot: LUMINE })
        );
        // A Growth star fits the Lumine slot, which has no category.
        assert_eq!(f.replace(1, LUMINE), Ok(()));
        assert_eq!(f.eq().lumine.as_ref().unwrap().held, Some(Held::Star(XP)));
        assert_eq!(f.eq().inventory[1], Held::Star(HP_C));
    }

    /// The starting rune in rune slot 0, as `loadout::apply` leaves it, with a cooldown running.
    fn thunder(f: &mut Fixture) {
        f.change(|e| {
            e.slots[9].held = Some(Held::Rune(Rune::Thunder));
            e.slots[9].level = 1;
        });
        f.s.players[PlayerIdx::SOLO].runes = vec![HeldRune {
            rune: Rune::Thunder,
            cooldown_left: 40,
        }];
    }

    fn runes(f: &Fixture) -> Vec<(Rune, u32)> {
        f.s.players[PlayerIdx::SOLO]
            .runes
            .iter()
            .map(|h| (h.rune, h.cooldown_left))
            .collect()
    }

    /// T17: runes go into the three rune slots and `runes` follows the slot order.
    #[test]
    fn runes_equip_into_the_three_rune_slots() {
        let mut f = fixture();
        thunder(&mut f);
        f.put(&[
            Held::Rune(Rune::Bleed),
            Held::Rune(Rune::Burn),
            Held::Rune(Rune::Poison),
            Held::Star(ATK),
        ]);
        assert_eq!(f.equip(0, RUNE2), Ok(()));
        assert_eq!(runes(&f), [(Rune::Thunder, 40), (Rune::Bleed, 0)]);
        // Slot 1 sits between the two: the list keeps the slot order.
        assert_eq!(f.equip(0, RUNE1), Ok(()));
        assert_eq!(
            runes(&f),
            [(Rune::Thunder, 40), (Rune::Burn, 0), (Rune::Bleed, 0)]
        );
        // There is no fourth rune slot.
        assert_eq!(
            f.equip(0, RUNE0),
            Err(SlotError::SlotFilled { slot: RUNE0 })
        );
        assert_eq!(f.equip(0, 12), Err(SlotError::NoSlot { slot: 12 }));
        assert_eq!(f.equip(0, 13), Err(SlotError::NoSlot { slot: 13 }));
        // A rune in a star slot, a star in a rune slot.
        assert_eq!(
            f.equip(0, ATTACK0),
            Err(SlotError::WrongSlot { slot: ATTACK0 })
        );
        assert_eq!(
            f.equip(1, RUNE0),
            Err(SlotError::SlotFilled { slot: RUNE0 })
        );
        assert_eq!(
            f.replace(1, RUNE0),
            Err(SlotError::WrongSlot { slot: RUNE0 })
        );
    }

    /// T18: a rune replace swaps in place; the same rune in two slots is refused.
    #[test]
    fn a_rune_replace_swaps_in_place() {
        let mut f = fixture();
        thunder(&mut f);
        f.put(&[Held::Rune(Rune::Bleed)]);
        assert_eq!(f.replace(0, RUNE0), Ok(()));
        assert_eq!(
            runes(&f),
            [(Rune::Bleed, 0)],
            "the old rune's cooldown is lost"
        );
        assert_eq!(f.eq().inventory, [Held::Rune(Rune::Thunder)]);
        f.put(&[Held::Rune(Rune::Bleed)]);
        assert_eq!(f.equip(1, RUNE1), Err(SlotError::RuneHeld { slot: RUNE0 }));
        assert_eq!(f.eq().inventory.len(), 2, "a refusal changes nothing");
    }

    /// T19: equipping and replacing draw nothing.
    #[test]
    fn equipping_draws_nothing_and_moves_no_stream() {
        let mut f = fixture();
        thunder(&mut f);
        f.put(&[
            Held::Star(ATK),
            Held::Star(HP_C),
            Held::Rune(Rune::Burn),
            Held::Star(CRIT),
        ]);
        let (rng, offer) = (
            f.s.rng.clone(),
            f.s.players.career(PlayerIdx::SOLO).offer.state(),
        );
        assert_eq!(f.equip(0, ATTACK0), Ok(()));
        assert_eq!(f.equip(0, DEFENCE0), Ok(()));
        assert_eq!(f.equip(0, RUNE1), Ok(()));
        assert_eq!(f.replace(0, ATTACK0), Ok(()));
        assert_eq!(f.s.rng, rng);
        assert_eq!(f.s.players.career(PlayerIdx::SOLO).offer.state(), offer);
    }
    /// Holds `place` in slot `at` at `level` and `expertise`, and writes the change.
    fn hold(f: &mut Fixture, at: usize, place: u16, level: u8, expertise: u32) {
        f.change(|e| {
            let slot = if at < e.slots.len() {
                &mut e.slots[at]
            } else {
                e.lumine.as_mut().unwrap()
            };
            slot.held = Some(Held::Star(place));
            slot.level = level;
            slot.expertise = expertise;
        });
    }

    fn lv(f: &Fixture, at: usize) -> (u8, u32) {
        let eq = f.eq();
        let s = eq.slots.get(at).or(eq.lumine.as_ref()).unwrap();
        (s.level, s.expertise)
    }

    /// T14: a replace by a star that shares no option keeps half the level, at least 1, no kills.
    #[test]
    fn a_replace_keeps_half_the_level_and_resets_the_kills() {
        for (level, kills, want) in [(7, 40, 3), (10, 0, 5), (2, 11, 1), (1, 5, 1)] {
            let mut f = fixture();
            hold(&mut f, 0, ATK, level, kills);
            f.put(&[Held::Star(CRIT)]);
            assert!(!slots::is_switch(&f.rewards, f.eq(), 0, ATTACK0));
            assert_eq!(f.replace(0, ATTACK0), Ok(()));
            assert_eq!(lv(&f, 0), (want, 0), "from Lv{level}");
            assert_eq!(f.eq().inventory, [Held::Star(ATK)]);
        }
    }

    /// T15: a same-option switch keeps the whole level and kills, and the build follows.
    #[test]
    fn a_same_option_switch_keeps_the_whole_level() {
        let mut f = fixture();
        let base = f.total(StatEffect::MaxHp);
        hold(&mut f, 5, HP_C, 7, 40);
        assert_eq!(
            f.total(StatEffect::MaxHp),
            base + slots::level_scaled(slots::bundled_rules(), 100, 7)
        );
        f.put(&[Held::Star(HP_R)]);
        assert!(slots::is_switch(&f.rewards, f.eq(), 0, DEFENCE0));
        assert_eq!(f.replace(0, DEFENCE0), Ok(()));
        assert_eq!(lv(&f, 5), (7, 40));
        assert_eq!(
            f.total(StatEffect::MaxHp),
            base + slots::level_scaled(slots::bundled_rules(), 250, 7)
        );
        // The Lumine slot switches the same way.
        let mut f = fixture();
        f.lumine(HP_C);
        hold(&mut f, LUMINE as usize, HP_C, 5, 9);
        f.put(&[Held::Star(HP_R)]);
        assert_eq!(f.replace(0, LUMINE), Ok(()));
        assert_eq!(lv(&f, LUMINE as usize), (5, 9));
    }

    /// T16: a replace rewrites the build from the old star at the old level to the new one at the
    /// new level.
    #[test]
    fn a_replace_changes_the_build_from_the_old_level_to_the_new() {
        let mut f = fixture();
        let (atk, hp) = (f.total(StatEffect::AttackPct), f.total(StatEffect::MaxHp));
        hold(&mut f, 0, ATK_HP, 10, 0);
        assert_eq!(
            f.total(StatEffect::AttackPct),
            atk + slots::level_scaled(slots::bundled_rules(), 4, 10)
        );
        f.put(&[Held::Star(CRIT)]);
        assert_eq!(f.replace(0, ATTACK0), Ok(()));
        assert_eq!(lv(&f, 0), (5, 0));
        assert_eq!(f.total(StatEffect::AttackPct), atk);
        assert_eq!(f.total(StatEffect::MaxHp), hp);
        assert_eq!(
            f.s.players[PlayerIdx::SOLO].crit_chance_pct,
            slots::level_scaled(slots::bundled_rules(), 5, 5)
        );
    }

    /// T17: a rune replace leaves the rune slot at Lv1.
    #[test]
    fn a_rune_replace_resets_to_level_1_at_most() {
        let mut f = fixture();
        thunder(&mut f);
        f.change(|e| e.slots[9].expertise = 3);
        f.put(&[Held::Rune(Rune::Bleed)]);
        assert_eq!(f.replace(0, RUNE0), Ok(()));
        assert_eq!(lv(&f, 9), (1, 0));
    }

    /// T18: an equip into an empty slot makes Lv1, keeps the kills and adds the base.
    #[test]
    fn an_equip_into_an_empty_slot_is_unchanged() {
        let mut f = fixture();
        f.change(|e| e.slots[0].expertise = 3);
        let atk = f.total(StatEffect::AttackPct);
        f.put(&[Held::Star(ATK)]);
        assert_eq!(f.equip(0, ATTACK0), Ok(()));
        assert_eq!(lv(&f, 0), (1, 3));
        assert_eq!(f.total(StatEffect::AttackPct), atk + 5);
    }
}

// --- expertise: the curve and the gain (ADR-SIM-3, ADR-SIM-5) ----------------------------------

mod curve {
    use super::super::slots::{self, MAX_LEVEL};

    /// T1: the curve rises and a slot takes 870 kills from Lv1 to Lv10.
    #[test]
    fn the_curve_rises_and_ends_at_lv10() {
        assert_eq!(slots::bundled_rules().kills_to_next.len(), 9);
        assert!(slots::bundled_rules().kills_to_next.iter().all(|&n| n > 0));
        assert!(
            slots::bundled_rules()
                .kills_to_next
                .windows(2)
                .all(|w| w[0] < w[1])
        );
        assert_eq!(
            slots::bundled_rules().kills_to_next.iter().sum::<u32>(),
            870
        );
        assert_eq!(MAX_LEVEL, 10);
    }

    /// T2: a slot rises one level at a time and keeps the rest.
    #[test]
    fn gain_levels_a_slot_one_level_at_a_time() {
        assert_eq!(slots::gain(slots::bundled_rules(), 1, 0, 9), (1, 9));
        assert_eq!(slots::gain(slots::bundled_rules(), 1, 9, 1), (2, 0));
        assert_eq!(slots::gain(slots::bundled_rules(), 1, 0, 35), (3, 5));
        assert_eq!(slots::gain(slots::bundled_rules(), 2, 19, 1), (3, 0));
        assert_eq!(slots::gain(slots::bundled_rules(), 9, 0, 230), (10, 0));
    }

    /// T3: Lv10 is the end: no overflow, no expertise kept.
    #[test]
    fn gain_stops_at_lv10() {
        assert_eq!(
            slots::gain(slots::bundled_rules(), 10, 0, 1_000_000),
            (10, 0)
        );
        assert_eq!(slots::gain(slots::bundled_rules(), 1, 0, u32::MAX), (10, 0));
        assert_eq!(slots::gain(slots::bundled_rules(), 9, 229, 5), (10, 0));
        assert_eq!(
            slots::gain(slots::bundled_rules(), 1, u32::MAX, u32::MAX),
            (10, 0)
        );
    }

    /// T5: a level adds a floored percentage of the base; Lv1 is the base.
    #[test]
    fn level_scaled_is_a_floored_percentage_of_the_base() {
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), 20_000, 1),
            20_000
        );
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), 20_000, 2),
            20_600
        );
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), 20_000, 10),
            25_400
        );
        assert_eq!(slots::level_scaled(slots::bundled_rules(), 5, 10), 6);
        assert_eq!(slots::level_scaled(slots::bundled_rules(), 5, 3), 5);
        assert_eq!(slots::level_scaled(slots::bundled_rules(), 0, 10), 0);
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), u32::MAX, 10),
            u32::MAX
        );
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), 7, 0),
            slots::level_scaled(slots::bundled_rules(), 7, 1)
        );
    }

    /// T4: a slot that never held anything gains nothing.
    #[test]
    fn an_empty_slot_gains_nothing() {
        assert_eq!(slots::gain(slots::bundled_rules(), 0, 0, 50), (0, 0));
    }
}

// --- kills give the killer's star slots expertise (ADR-SIM-3, ADR-SIM-7) -----------------------

mod kills {
    use super::super::equipment::{EquipSlot, Held, SlotKind};
    use super::super::slots::{self};
    use super::super::state::{AgentKey, AgentSource, PlayerIdx, State};
    use super::grants::{Fixture, rarities, star, with};
    use crate::fixed::Point;
    use crate::id::EntityId;
    use crate::rewards::Rewards;
    use crate::run_content::RunContent;
    use crate::run_stats::StatEffect;
    use crate::version::Versions;
    use serde_json::json;

    const ATK: u16 = 0;
    const HP: u16 = 1;
    const SPEED: u16 = 2;

    fn rewards() -> Rewards {
        let list = vec![
            star(
                "atk",
                "common",
                vec![json!({ "effect": "attack_pct", "amount": 100 })],
            ),
            star(
                "hp",
                "common",
                vec![json!({ "effect": "max_hp", "amount": 20_000 })],
            ),
            star(
                "speed",
                "common",
                vec![json!({ "effect": "speed_pct", "amount": 5 })],
            ),
        ];
        let mut file = json!({ "rarities": rarities(), "rewards": list });
        crate::sim::starlight::with_equipment_rules(&mut file);
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("validates")
    }

    /// An equipment solo fixture with the attack star in slot 0 and the hp star in the Lumine slot.
    fn armed() -> Fixture {
        let mut f = with(rewards());
        f.stage.equipment_mode = true;
        f.stage.equipment_rules = slots::bundled_rules().clone();
        f.set(0, Some(ATK));
        f.change(|e| {
            e.lumine = Some(EquipSlot {
                kind: SlotKind::Lumine,
                held: Some(Held::Star(HP)),
                level: 1,
                expertise: 0,
            });
        });
        f
    }

    fn slot_of(f: &Fixture, who: PlayerIdx, at: usize) -> (u8, u32) {
        let eq = &f.s.players.career(who).equipment;
        let s = eq.slots.get(at).or(eq.lumine.as_ref()).unwrap();
        (s.level, s.expertise)
    }

    /// `n` enemies at 0 hit points, the first `burst` of them a kill burst's.
    fn dead(f: &mut Fixture, n: u32, burst: u32) {
        let key = AgentKey {
            source: AgentSource::Wave,
            index: 0,
            rule: 0,
        };
        for i in 0..n {
            let mut e = super::super::agents::new_agent(
                EntityId(100 + i),
                f.stage.defs.of_key(key),
                f.stage.defs.get(f.stage.defs.of_key(key)),
                key,
                Point::default(),
                0,
            );
            e.hp = 0;
            e.burst = i < burst;
            f.s.enemies.push(e);
        }
    }

    /// The tick's two steps: `collide` counts the kills, `credit_kills` spends them.
    fn tick(f: &mut Fixture, killer: PlayerIdx) {
        for i in 0..f.s.enemies.len() {
            super::super::record_kill(&mut f.s, i, killer);
        }
        let mut ev = Vec::new();
        super::super::collide(&f.stage, &mut f.s, false, &mut ev);
        slots::credit_kills(&f.stage, &f.rewards, &mut f.s);
    }

    /// T9: a kill gives every star slot of its killer a point; rune and empty slots gain nothing.
    #[test]
    fn a_kill_credits_the_killer_s_star_slots() {
        let mut f = armed();
        let atk = f.total(StatEffect::AttackPct);
        dead(&mut f, 10, 0);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 0), (2, 0));
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 12), (2, 0), "the Lumine slot");
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 1), (0, 0), "an empty slot");
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 9), (0, 0), "a rune slot");
        assert_eq!(f.total(StatEffect::AttackPct), atk + 3, "3 % of 100");
        assert_eq!(f.s.enemies.len(), 0);
    }

    /// T10: each enemy counts once, a burst's never, an enemy alive not at all; the count is emptied.
    #[test]
    fn a_kill_counts_once_per_enemy_and_never_a_burst() {
        let mut f = armed();
        dead(&mut f, 3, 1);
        let mut alive = f.s.enemies[0].clone();
        alive.hp = 1_000_000;
        alive.burst = false;
        alive.id = EntityId(900);
        f.s.enemies.push(alive);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 0), (1, 2));
        assert!(f.s.scratch.kills_by.iter().all(|&k| k == 0));
    }

    /// T11: kills go to the player who made them; the others' slots and builds stay.
    #[test]
    fn kills_go_to_the_player_who_made_them() {
        let mut f = armed();
        let one = PlayerIdx::new(1).unwrap();
        let two = PlayerIdx::new(2).unwrap();
        f.s.players =
            super::super::state::Players::with_mask(0b111, f.s.players[PlayerIdx::SOLO].clone())
                .keeping_career_of(&f.s.players);
        for who in [one, two] {
            let eq = &mut f.s.players.career_mut(who).equipment;
            eq.slots = slots::fresh_slots(slots::bundled_rules());
            eq.slots[0].held = Some(Held::Star(SPEED));
            eq.slots[0].level = 1;
        }
        let other = f.total(StatEffect::SpeedPct);
        dead(&mut f, 10, 0);
        tick(&mut f, one);
        assert_eq!(slot_of(&f, one, 0), (2, 0));
        assert_eq!(slot_of(&f, two, 0), (1, 0));
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 0), (1, 0));
        assert_eq!(f.total(StatEffect::SpeedPct), other);
    }

    /// T12: the 10th kill is the one that changes the build; 9 change nothing.
    #[test]
    fn a_level_up_changes_the_build_in_the_tick() {
        let mut f = armed();
        let atk = f.total(StatEffect::AttackPct);
        dead(&mut f, slots::bundled_rules().kills_to_next[0] - 1, 0);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(f.total(StatEffect::AttackPct), atk);
        dead(&mut f, 1, 0);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(
            f.total(StatEffect::AttackPct),
            atk + slots::level_scaled(slots::bundled_rules(), 100, 2) - 100
        );
    }

    /// T13: an offers run credits nothing and counts nothing.
    #[test]
    fn offers_mode_credits_nothing() {
        let mut f = armed();
        f.stage.equipment_mode = false;
        dead(&mut f, 12, 0);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 0), (1, 0));
        assert!(f.s.scratch.kills_by.iter().all(|&k| k == 0));
    }

    /// The bundled Mintaka (`max_hp` 50,000 and `regen` 3,000) in the Lumine slot of a fixture
    /// over the bundled stars, and its place.
    fn bundled_with_lumine() -> (Fixture, u16) {
        let rewards = RunContent::bundled().rewards().clone();
        let place = u16::try_from(rewards.index_of(&"star_mintaka".into()).unwrap()).unwrap();
        let mut f = with(rewards);
        f.change(|e| {
            e.lumine = Some(EquipSlot {
                kind: SlotKind::Lumine,
                held: Some(Held::Star(place)),
                level: 1,
                expertise: 0,
            });
        });
        (f, place)
    }

    /// T4 (Lumine): ten kills raise the Lumine star to Lv2 and its build with it; nine do not; a
    /// category slot holding the same star rises at the same kill. The kills also level the
    /// player, so the build is read against a run with no star that has the same kills.
    #[test]
    fn kills_raise_the_lumine_slot_like_any_other() {
        let regen = |f: &Fixture| f.s.players.career(PlayerIdx::SOLO).regen;
        let (mut f, _) = bundled_with_lumine();
        let mut control = with(RunContent::bundled().rewards().clone());
        let gap = |f: &Fixture, c: &Fixture| {
            (
                f.total(StatEffect::MaxHp) - c.total(StatEffect::MaxHp),
                regen(f) - regen(c),
            )
        };
        assert_eq!(gap(&f, &control), (50_000, 3_000));
        for fx in [&mut f, &mut control] {
            dead(fx, slots::bundled_rules().kills_to_next[0] - 1, 0);
            tick(fx, PlayerIdx::SOLO);
        }
        assert_eq!(
            slot_of(&f, PlayerIdx::SOLO, 12),
            (1, slots::bundled_rules().kills_to_next[0] - 1)
        );
        assert_eq!(
            gap(&f, &control),
            (50_000, 3_000),
            "nine kills change nothing"
        );
        for fx in [&mut f, &mut control] {
            dead(fx, 1, 0);
            tick(fx, PlayerIdx::SOLO);
        }
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 12), (2, 0));
        assert_eq!(
            slots::level_scaled(slots::bundled_rules(), 50_000, 2),
            51_500
        );
        assert_eq!(gap(&f, &control), (51_500, 3_090));

        let (mut f, place) = bundled_with_lumine();
        f.change(|e| {
            e.slots[0].held = Some(Held::Star(place));
            e.slots[0].level = 1;
        });
        dead(&mut f, slots::bundled_rules().kills_to_next[0], 0);
        tick(&mut f, PlayerIdx::SOLO);
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 0), (2, 0));
        assert_eq!(slot_of(&f, PlayerIdx::SOLO, 12), (2, 0));
    }

    #[allow(dead_code)]
    fn _uses(_: &State) {}
}

// --- the Lumine star against the bundled stars (ADR-SAVE-7, ADR-SIM-7) -------------------------

mod bundled_lumine {
    use super::*;
    use crate::run_content::RunContent;

    fn run(lumine: u8, star: Option<&str>) -> Engine {
        Engine::start(&RunSetup {
            build_mode: BuildMode::Equipment,
            lumine,
            lumine_star: star.map(Into::into),
            ..RunSetup::default()
        })
        .expect("starts")
    }

    fn total(s: &State, stat: StatEffect) -> u32 {
        s.players
            .career(PlayerIdx::SOLO)
            .progress
            .as_ref()
            .expect("levels")
            .totals[stat as usize]
    }

    /// T1: a Lumine star with regen is in the build at tick 0, at Lv1, and draws nothing.
    #[test]
    fn a_lumine_star_with_regen_is_in_the_build_at_tick_0() {
        let (plain, with) = (run(4, None), run(4, Some("star_mintaka")));
        let (p, s) = (state_of(&plain), state_of(&with));
        let (was, me) = (&p.players[PlayerIdx::SOLO], &s.players[PlayerIdx::SOLO]);
        assert_eq!(me.max_hp, was.max_hp + 50_000);
        assert_eq!(me.hp, me.max_hp);
        assert_eq!(
            total(s, StatEffect::MaxHp),
            total(p, StatEffect::MaxHp) + 50_000
        );
        let (cs, cp) = (
            s.players.career(PlayerIdx::SOLO),
            p.players.career(PlayerIdx::SOLO),
        );
        assert_eq!(cs.regen, cp.regen + 3_000);
        let slot = cs.equipment.lumine.as_ref().expect("the Lumine slot");
        assert!(matches!(slot.held, Some(Held::Star(_))));
        assert_eq!(
            (slot.kind, slot.level, slot.expertise),
            (SlotKind::Lumine, 1, 0)
        );
        assert_eq!(cs.offer.state(), cp.offer.state(), "no offer draw");
        assert_eq!(s.rng.loot.state(), p.rng.loot.state(), "no loot draw");
    }

    /// T2: a Lumine set member counts at tick 0 (one of three, threshold 2: no tier), with its speed.
    #[test]
    fn a_lumine_set_member_counts_at_tick_0() {
        let (plain, with) = (run(4, None), run(4, Some("star_altair")));
        let (p, s) = (state_of(&plain), state_of(&with));
        let offers = |s: &State| {
            s.players
                .career(PlayerIdx::SOLO)
                .offers
                .clone()
                .expect("offers")
        };
        let (po, so) = (offers(p), offers(s));
        let at = RunContent::bundled()
            .rewards()
            .constellations()
            .set_of(&"star_altair".into())
            .expect("the set");
        assert_eq!(so.set_counts[at], po.set_counts[at] + 1);
        assert_eq!(so.set_counts[at], 1, "below the threshold of 2");
        // The bundled stage names no `max_charges`, so the star's charge is clamped to the
        // stage's own count (`player.rs`): the star is held, the count does not move.
        assert_eq!(
            s.players[PlayerIdx::SOLO].evade_charges,
            p.players[PlayerIdx::SOLO].evade_charges
        );
        assert_eq!(
            total(s, StatEffect::SpeedPct),
            total(p, StatEffect::SpeedPct) + 20
        );
    }

    /// T3: a Lumine orbit is in the player's orbits at tick 0, ready.
    #[test]
    fn a_lumine_orbit_arrives_ready_at_tick_0() {
        let (plain, with) = (run(10, None), run(10, Some("star_sirius")));
        let (p, s) = (state_of(&plain), state_of(&with));
        assert!(p.players[PlayerIdx::SOLO].orbits.is_empty());
        let orbits = &s.players[PlayerIdx::SOLO].orbits;
        assert_eq!(orbits.len(), 1);
        assert_eq!((orbits[0].orbs, orbits[0].cooldown_left), (4, 0));
        assert_eq!(
            total(s, StatEffect::AttackPct),
            total(p, StatEffect::AttackPct) + 30
        );
    }
}
