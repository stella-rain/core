//! SplitMix64 is the simulation PRNG (ADR-033). Its outputs are frozen rules: these vectors
//! come from the published algorithm and must never change.

use std::collections::BTreeMap;

use serde_json::json;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rng::{
    SplitMix64, TAG_ATTACK, TAG_HORDE, TAG_LOOT, TAG_OFFER, TAG_SPAWN, derive,
};
use stella_rain_core::snapshot::EntityKind;

use crate::common;

use common::*;

#[test]
fn seed_zero_gives_the_published_sequence() {
    let mut rng = SplitMix64::new(0);
    assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
    assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
}

#[test]
fn a_nonzero_seed_gives_the_pinned_sequence() {
    let mut rng = SplitMix64::new(0x5354_454c_4c41_5241);
    assert_eq!(rng.next_u64(), 0x424b_882e_8174_a4df);
    assert_eq!(rng.next_u64(), 0x5dc2_2b34_0f92_a76d);
}

#[test]
fn below_is_next_u64_modulo_n() {
    let mut a = SplitMix64::new(7);
    let mut b = SplitMix64::new(7);
    for n in [1_u32, 2, 3, 100, 4_000, u32::MAX] {
        let expected = u32::try_from(b.next_u64() % u64::from(n)).unwrap();
        assert_eq!(a.below(n), expected);
    }
}

#[test]
#[should_panic(expected = "below(0)")]
fn below_zero_panics() {
    SplitMix64::new(1).below(0);
}

/// The first three outputs of the stream `tag` derives from seed 42.
fn first_three(tag: u64) -> [u64; 3] {
    let mut rng = derive(42, tag);
    [rng.next_u64(), rng.next_u64(), rng.next_u64()]
}

#[test]
fn each_stream_is_pinned() {
    // The tags are the stream's name padded with zeros to 8 bytes, read little-endian.
    assert_eq!(TAG_SPAWN, 0x006e_7761_7073);
    assert_eq!(TAG_ATTACK, 0x6b63_6174_7461);
    assert_eq!(TAG_LOOT, 0x746f_6f6c);
    assert_eq!(TAG_OFFER, 0x0072_6566_666f);
    assert_eq!(TAG_HORDE, 0x0065_6472_6f68);
    assert_eq!(
        first_three(TAG_SPAWN),
        [
            0xb8e0_7551_93f7_b5c5,
            0x95fa_578b_2e68_42a1,
            0x2e0b_3901_a270_293d
        ]
    );
    assert_eq!(
        first_three(TAG_ATTACK),
        [
            0x3ebe_4b3d_31d1_dd59,
            0x59ef_da98_b95c_32ca,
            0xec2e_ade6_bb5d_8b19
        ]
    );
    assert_eq!(
        first_three(TAG_LOOT),
        [
            0x972f_003d_00a9_c7d7,
            0x4b48_bebc_8f2a_4944,
            0x6ad9_03ae_850a_9210
        ]
    );
    assert_eq!(
        first_three(TAG_OFFER),
        [
            0xffc3_1cca_3dd4_9103,
            0x5dc8_971b_d9fe_29af,
            0x0ddb_b6e7_9640_a697
        ]
    );
    assert_eq!(
        first_three(TAG_HORDE),
        [
            0x490c_5156_39d4_4986,
            0xeb00_3f6a_972f_f0e6,
            0x27f2_0b45_ee4c_9f71
        ]
    );
}

/// Where each enemy of a ring wave first stands, in spawn order, over 600 ticks of a run in which
/// the main character kills them, with a heal pickup dropping one in `drop_one_in`.
fn wave_positions(drop_one_in: Option<u32>) -> Vec<(i32, i32)> {
    let weapon = json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } });
    let mut ring = wave(
        1,
        100,
        6,
        enemy(25, hold(START_X, START_Y), json!([])),
        START_X,
        START_Y,
    );
    ring.as_object_mut().unwrap().remove("spawn");
    ring["edge"] = json!({ "side": "any" });
    let mut stage = armed(
        stage_full(1_800, Some(far_boss()), vec![ring], vec![]),
        weapon,
    );
    stage.player.hp = 1_000;
    stage.heal = drop_one_in.map(|one_in| {
        serde_json::from_value(json!({
            "boss_pct": 25, "drop_one_in": one_in, "amount": 30_000, "cap": 16,
            "pickup_radius": 12_288
        }))
        .unwrap()
    });
    let mut e = engine(&stage);
    let mut seen = BTreeMap::new(); // by entity ID, which keeps the spawn order
    let mut kills = 0;
    for _ in 0..600 {
        e.step(firing(idle(), e.tick()));
        kills += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::Died { .. }))
            .count();
        for v in e.snapshot().entities {
            if v.kind == EntityKind::Enemy {
                seen.entry(v.id.0).or_insert((v.at.x.0, v.at.y.0));
            }
        }
    }
    // The loot stream really draws: enemies of the wave die in the run.
    assert!(kills > 0, "no enemy died");
    seen.into_values().collect()
}

#[test]
fn a_loot_change_leaves_spawn_angles() {
    let often = wave_positions(Some(2));
    let rare = wave_positions(Some(3));
    assert!(often.len() >= 90, "the ring wave spawned {}", often.len());
    assert_eq!(often, rare);
}

#[test]
fn a_run_without_heal_drops_has_the_same_spawn_angles() {
    // A heal config draws once per kill from the loot stream; without one nothing is drawn.
    assert_eq!(wave_positions(None), wave_positions(Some(2)));
}
