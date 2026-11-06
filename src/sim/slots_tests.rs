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

    fn star(id: &str, effects: Vec<Value>) -> Value {
        json!({ "id": id, "rarity": "common", "limit": 3, "effects": effects })
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

    fn rewards() -> Rewards {
        let rarities: Vec<Value> = [
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
        .collect();
        let list = vec![
            star("hp", vec![json!({ "effect": "max_hp", "amount": 100 })]),
            star("nova", vec![skill("nova", 1)]),
            star("war_cry", vec![skill("war_cry", 1)]),
            star("nova2", vec![skill("nova", 2)]),
            star("orbit_a", vec![orbit(2, 120)]),
            star("orbit_b", vec![orbit(3, 90)]),
            star(
                "evade_a",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "evade_b",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "evade_c",
                vec![json!({ "effect": "evade_charge", "amount": 1 })],
            ),
            star(
                "reroll",
                vec![
                    json!({ "effect": "rerolls", "amount": 2 }),
                    json!({ "effect": "banishes", "amount": 1 }),
                ],
            ),
            star(
                "bundle",
                vec![
                    json!({ "effect": "random_rune" }),
                    json!({ "effect": "attack_pct", "amount": 3 }),
                ],
            ),
        ];
        let file = json!({ "rarities": rarities, "rewards": list });
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("validates")
    }

    struct Fixture {
        stage: World,
        rewards: Rewards,
        s: State,
    }

    fn fixture() -> Fixture {
        let mut stage = RunContent::bundled().trial().clone();
        stage.player.evade.charges = 1;
        stage.player.evade.max_charges = Some(3);
        let stage = World::for_test(stage);
        let pool = RunPool {
            rewards: (0..11).collect(),
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
        Fixture {
            stage,
            rewards: rewards(),
            s,
        }
    }

    impl Fixture {
        /// Puts `place` (or nothing) into slot `at` and writes the change.
        fn set(&mut self, at: usize, place: Option<u16>) {
            self.change(|e| {
                e.slots[at].held = place.map(Held::Star);
                e.slots[at].level = u8::from(place.is_some());
            });
        }

        fn change(&mut self, f: impl FnOnce(&mut Equipment)) {
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
