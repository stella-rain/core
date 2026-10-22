//! The bundled named stars and Constellations (core#NS4, app#336): 15 named stars (6 epic, 6
//! legendary, 3 supernova), each taken once and drawn at half an ordinary reward's weight, and
//! four sets of three, each with a small tier at 2 and the full tier at 3. Rigel, Antares and
//! Polaris are in no set. The numbers are the options file's first ideas, tuned by play.

use stella_rain_core::difficulties::Difficulties;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::ContentId;
use stella_rain_core::pack::Pack;
use stella_rain_core::rewards::{DEFAULT_WEIGHT, Rarity, Reward, Rewards};
use stella_rain_core::run_content::RunContent;

use crate::reward_kinds::{near, next_offer, offer};

fn rewards() -> &'static Rewards {
    Pack::bundled().content().rewards()
}

fn named() -> Vec<&'static Reward> {
    rewards().rewards().iter().filter(|r| r.named).collect()
}

fn id(s: &str) -> ContentId {
    ContentId(s.into())
}

#[test]
fn the_bundled_pack_holds_fifteen_named_stars_six_epic_six_legendary_three_supernova() {
    let stars = named();
    assert_eq!(stars.len(), 15);
    for (rarity, count) in [
        (Rarity::Epic, 6),
        (Rarity::Legendary, 6),
        (Rarity::Supernova, 3),
    ] {
        let found = stars.iter().filter(|r| r.rarity == rarity).count();
        assert_eq!(found, count, "{rarity:?}");
    }
    for r in stars {
        assert_eq!(r.limit, Some(1), "{}", r.id.0);
        assert_eq!(r.weight, DEFAULT_WEIGHT / 2, "{}", r.id.0);
        assert!(r.star.is_some(), "{}", r.id.0);
    }
}

#[test]
fn the_four_constellations_have_their_members_and_two_tiers() {
    let sets = Pack::bundled().content().constellations().sets();
    let expected = [
        (
            "summer_triangle",
            ["star_altair", "star_deneb", "star_vega"],
        ),
        (
            "orions_belt",
            ["star_alnitak", "star_alnilam", "star_mintaka"],
        ),
        ("big_dipper", ["star_dubhe", "star_mizar", "star_alkaid"]),
        (
            "winter_triangle",
            ["star_procyon", "star_sirius", "star_betelgeuse"],
        ),
    ];
    assert_eq!(sets.len(), 4);
    for (set, (set_id, members)) in sets.iter().zip(expected) {
        assert_eq!(set.id.0, set_id);
        let found: Vec<&str> = set.members.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(found, members, "{set_id}");
        let thresholds: Vec<u8> = set.tiers.iter().map(|t| t.threshold).collect();
        assert_eq!(thresholds, [2, 3], "{set_id}");
    }
}

#[test]
fn rigel_antares_and_polaris_are_in_no_set() {
    let sets = Pack::bundled().content().constellations();
    for star in ["star_rigel", "star_antares", "star_polaris"] {
        assert!(
            rewards().index_of(&id(star)).is_some(),
            "{star} is a reward"
        );
        assert_eq!(sets.set_of(&id(star)), None, "{star}");
    }
}

#[test]
fn a_normal_run_can_be_offered_a_named_star() {
    let mut seen = false;
    for seed in 1..=20 {
        let mut stage = near(40);
        stage.seed = seed;
        stage.stars.as_mut().unwrap().tiers = serde_json::from_value(serde_json::json!(
            [{ "from_level": 1, "weights": { "epic": 1 } }]
        ))
        .unwrap();
        let mut e = Engine::new_with(&stage, RunContent::bundled()).unwrap();
        next_offer(&mut e);
        seen |= offer(&e).candidates.iter().any(|c| c.named);
    }
    assert!(seen, "no named star was offered in 20 seeded runs");
}

#[test]
fn easy_offers_no_named_star_by_its_rarity_range() {
    let easy = Difficulties::bundled()
        .get(&id("easy"))
        .expect("easy is bundled");
    let lowest = named().iter().map(|r| r.rarity).min().unwrap();
    assert!(
        easy.rarity_max < lowest,
        "easy reaches {:?}",
        easy.rarity_max
    );
    let normal = Difficulties::bundled().get(&id("normal")).unwrap();
    assert!(normal.rarity_max >= lowest);
}

#[test]
fn taking_a_random_rune_bundle_gives_the_rune_and_the_haste() {
    let list = vec![crate::reward_kinds::kind(
        "dog",
        vec![
            serde_json::json!({ "effect": "random_rune" }),
            crate::reward_kinds::entry("rune_haste", 100),
        ],
        1_000,
        2,
    )];
    let mut e = crate::reward_kinds::with(&near(40), list);
    next_offer(&mut e);
    crate::reward_kinds::take(&mut e, "dog");
    let player = e.snapshot().player;
    assert_eq!(player.runes.len(), 1);
    assert_eq!(player.rune_haste_pct, 100);
}

#[test]
fn the_bundled_rewards_file_validates_with_its_named_stars() {
    // The file itself, before it is copied into the pack: every cap of NS1 and NS2 applies.
    let rewards = Rewards::parse(Rewards::BUNDLED).unwrap_or_else(|e| panic!("{e}"));
    let stars: Vec<_> = rewards.rewards().iter().filter(|r| r.named).collect();
    assert_eq!(stars.len(), 15);
    assert_eq!(rewards.rewards().len(), 62);
}
