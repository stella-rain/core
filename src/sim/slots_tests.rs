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
        ];
        let file = json!({ "rarities": rarities, "rewards": list });
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
        let mut stage = RunContent::bundled().trial().clone();
        stage.player.evade.charges = 1;
        stage.player.evade.max_charges = Some(3);
        let stage = World::for_test(stage);
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
        s.players.career_mut(PlayerIdx::SOLO).equipment.slots = slots::fresh_slots();
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
            let before: Vec<u16> = slots::equipped(&career.equipment).collect();
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

    /// T10: rerolls and banishes add and a replace removes exactly their amounts; a random rune in
    /// a bundle draws nothing.
    #[test]
    fn rerolls_banishes_and_the_random_rune_grant_nothing_else() {
        let mut f = fixture();
        let o = |f: &Fixture| {
            let o = f.s.players.career(PlayerIdx::SOLO).offers.as_ref().unwrap();
            (o.rerolls_left, o.banishes_left)
        };
        let base = o(&f);
        f.set(0, Some(REROLL));
        assert_eq!(o(&f), (base.0 + 2, base.1 + 1));
        f.set(0, None);
        assert_eq!(o(&f), base);

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
        let file = json!({ "rarities": rarities(), "rewards": list });
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

        fn total(&self, stat: StatEffect) -> u32 {
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

    /// T12: a replace puts the displaced star at the item's place; level and expertise stay.
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
        assert_eq!((f.eq().slots[0].level, f.eq().slots[0].expertise), (3, 17));
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
}

// --- expertise: the curve and the gain (ADR-SIM-3, ADR-SIM-5) ----------------------------------

mod curve {
    use super::super::slots::{self, KILLS_TO_NEXT, MAX_LEVEL};

    /// T1: the curve rises and a slot takes 870 kills from Lv1 to Lv10.
    #[test]
    fn the_curve_rises_and_ends_at_lv10() {
        assert_eq!(KILLS_TO_NEXT.len(), 9);
        assert!(KILLS_TO_NEXT.iter().all(|&n| n > 0));
        assert!(KILLS_TO_NEXT.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(KILLS_TO_NEXT.iter().sum::<u32>(), 870);
        assert_eq!(MAX_LEVEL, 10);
    }

    /// T2: a slot rises one level at a time and keeps the rest.
    #[test]
    fn gain_levels_a_slot_one_level_at_a_time() {
        assert_eq!(slots::gain(1, 0, 9), (1, 9));
        assert_eq!(slots::gain(1, 9, 1), (2, 0));
        assert_eq!(slots::gain(1, 0, 35), (3, 5));
        assert_eq!(slots::gain(2, 19, 1), (3, 0));
        assert_eq!(slots::gain(9, 0, 230), (10, 0));
    }

    /// T3: Lv10 is the end: no overflow, no expertise kept.
    #[test]
    fn gain_stops_at_lv10() {
        assert_eq!(slots::gain(10, 0, 1_000_000), (10, 0));
        assert_eq!(slots::gain(1, 0, u32::MAX), (10, 0));
        assert_eq!(slots::gain(9, 229, 5), (10, 0));
        assert_eq!(slots::gain(1, u32::MAX, u32::MAX), (10, 0));
    }

    /// T4: a slot that never held anything gains nothing.
    #[test]
    fn an_empty_slot_gains_nothing() {
        assert_eq!(slots::gain(0, 0, 50), (0, 0));
    }
}
