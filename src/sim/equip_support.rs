//! Stars equipped at a level on a `State`, for the tests of `src/sim` (ADR-SIM-7): the twin of
//! `tests/common/equip.rs`, which does the same through the engine's actions. Tests import
//! `equip_stars` in place of repeating the set-then-`refresh` steps by hand.

use super::state::{PlayerIdx, State};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, Run};
    use crate::id::ContentId;
    use crate::input::EquipAction;
    use crate::pack::ContentFiles;
    use crate::rewards::Rewards;
    use crate::run_content::RunContent;
    use crate::run_setup::{BuildMode, RunSetup};
    use crate::sim::slots;
    use crate::version::{SIM_VERSION, Versions};
    use serde_json::json;

    /// Two stars of two categories (`crit_chance_pct`: Attack, slot 0; `max_hp`: Defence,
    /// slot 5), with the bundled rules and rarities.
    fn rewards() -> Rewards {
        let bundled: serde_json::Value =
            serde_json::from_slice(ContentFiles::bundled_file("rewards")).unwrap();
        let star = |id: &str, effect: &str, amount: u32| {
            json!({ "id": id, "rarity": "common", "limit": 3,
                    "effects": [{ "effect": effect, "amount": amount }] })
        };
        let mut file = json!({
            "rarities": bundled["rarities"],
            "rewards": [star("crit", "crit_chance_pct", 40), star("hp", "max_hp", 100)],
        });
        slots_rules(&mut file);
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap()
    }

    fn slots_rules(file: &mut serde_json::Value) {
        crate::sim::starlight::with_equipment_rules(file);
    }

    /// An equipment engine over `rewards()` on the bundled trial stage, nothing equipped.
    fn engine() -> (Engine, Rewards) {
        let rewards = rewards();
        let content = RunContent::new(RunContent::bundled().run_stats().clone(), rewards.clone());
        let mut stage = RunContent::bundled().trial().clone();
        stage.sim_version = SIM_VERSION;
        let setup = RunSetup {
            build_mode: BuildMode::Equipment,
            star_pool: Some(vec!["crit".into(), "hp".into()]),
            ..RunSetup::default()
        };
        (
            Engine::new_equipped(&stage, &content, &setup).unwrap(),
            rewards,
        )
    }

    /// T8: `equip_stars` writes what the engine's own `Equip` does, at level 1 and at level 3.
    #[test]
    fn the_twin_equals_the_action_path() {
        for level in [1u8, 3] {
            let (mut acted, rewards) = engine();
            let me = PlayerIdx::SOLO;
            let ids: Vec<ContentId> = vec!["crit".into(), "hp".into()];
            acted
                .script_equipment(me, &ids, &[(0, level), (5, level)])
                .unwrap();
            for slot in [0, 5] {
                acted.act(me, EquipAction::Equip { item: 0, slot }).unwrap();
            }

            let (mut twin, _) = engine();
            let Run::V2 { world, state } = twin.run_mut() else {
                panic!("the live rules");
            };
            equip_stars(world, &rewards, state, me, &[(0, 0), (1, 5)], level);
            assert_eq!(twin.state_hash(), acted.state_hash(), "Lv{level}");
        }
    }

    /// T9: `level_scaled` is the restated formula (0 reads as 1, one past the top as the top),
    /// and an equip into a slot already at level n keeps n, the rule `script_equipment` rests on.
    #[test]
    fn level_scaled_is_the_restated_formula() {
        let rules = slots::bundled_rules();
        for amount in [1u32, 7, 100] {
            for level in 0..=11u8 {
                let n = u32::from(level.clamp(1, 10));
                let want = amount * (100 + rules.level_pct * (n - 1)) / 100;
                assert_eq!(
                    slots::level_scaled(rules, amount, level),
                    want,
                    "{amount} Lv{level}"
                );
            }
        }
        let (mut e, _) = engine();
        let me = PlayerIdx::SOLO;
        e.script_equipment(me, &["crit".into()], &[(0, 4)]).unwrap();
        e.act(me, EquipAction::Equip { item: 0, slot: 0 }).unwrap();
        assert_eq!(e.snapshot().players[0].slots[0].level, 4);
    }
}
