//! The constellations (core#NS3, app#336): sets of named stars with tiers. The file is read as
//! untrusted bytes (ADR-SVC-1): members must be named stars of the run's rewards, each in at most
//! one set, 2 to 4 to a set; the tiers ascend and the top one is the whole set; a tier's
//! effects are the reward effects, held to the same caps. A tier lists the effects the set
//! has **at that tier**: reaching the full tier replaces the small one's, it is not added to
//! it. The engine counts the members held after every take, carried stars included.

use serde_json::{Value, json};
use stella_rain_core::constellations::{
    Constellations, MAX_CONSTELLATIONS, MAX_CONSTELLATIONS_BYTES,
};
use stella_rain_core::engine::Engine;
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::rewards::Rewards;
use stella_rain_core::validate::Error;
use stella_rain_core::version::{SIM_VERSION, Versions};

use crate::reward_kinds::{near, plain};

fn named(id: &str, star: &str) -> Value {
    json!({ "id": id, "rarity": "epic", "named": true, "star": star, "limit": 1,
            "effects": [{ "effect": "attack_pct", "amount": 1 }] })
}

/// Three named stars of one trio, and two more, and four commons.
fn reward_list() -> Vec<Value> {
    vec![
        named("n1", "Alpha"),
        named("n2", "Beta"),
        named("n3", "Gamma"),
        named("n4", "Delta"),
        named("n5", "Epsilon"),
        plain("a", 1),
        plain("b", 1),
        plain("c", 1),
        plain("d", 1),
    ]
}

fn rewards_of(list: Vec<Value>) -> Rewards {
    let file = json!({ "rarities": crate::common::rarities(), "rewards": list });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate")
}

fn tier(threshold: u8, effects: Vec<Value>) -> Value {
    json!({ "threshold": threshold, "effects": effects })
}

fn rerolls(amount: u32) -> Value {
    json!({ "effect": "rerolls", "amount": amount })
}

fn max_hp(amount: u32) -> Value {
    json!({ "effect": "max_hp", "amount": amount })
}

fn set(id: &str, members: &[&str], tiers: Vec<Value>) -> Value {
    json!({ "id": id, "name": format!("The {id}"), "members": members, "tiers": tiers })
}

/// A trio: 1 reroll at two of three, 3 at all three.
fn trio() -> Value {
    set(
        "trio",
        &["n1", "n2", "n3"],
        vec![tier(2, vec![rerolls(1)]), tier(3, vec![rerolls(3)])],
    )
}

fn file(sets: Vec<Value>) -> Value {
    json!({ "constellations": sets })
}

fn parse_at(v: &Value, versions: Versions) -> Result<Constellations, Error> {
    Constellations::parse_with(
        v.to_string().as_bytes(),
        versions,
        &rewards_of(reward_list()),
    )
}

fn parse(v: &Value) -> Result<Constellations, Error> {
    parse_at(v, Versions::CURRENT)
}

fn invalid_at(v: &Value, path: &str) {
    match parse(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

// --- The file ----------------------------------------------------------------------------

#[test]
fn a_valid_file_loads_with_its_sets_and_hash() {
    let c = parse(&file(vec![trio()])).expect("valid");
    assert_eq!(c.sets().len(), 1);
    assert_eq!(c.sets()[0].id.as_str(), "trio");
    assert_eq!(c.sets()[0].members.len(), 3);
    assert_eq!(c.sets()[0].tiers.len(), 2);
    assert!(!c.is_empty());
    let other = parse(&file(vec![set(
        "trio",
        &["n1", "n2", "n3"],
        vec![tier(3, vec![rerolls(3)])],
    )]))
    .unwrap();
    assert_ne!(c.sha256(), other.sha256());
}

#[test]
fn an_empty_list_is_valid_and_means_no_sets() {
    let c = parse(&file(vec![])).expect("no sets is a file too");
    assert!(c.is_empty());
    assert!(Constellations::default().is_empty());
}

#[test]
fn members_are_rewards_of_the_file_and_named() {
    let at = "$.constellations[0].members[1]";
    invalid_at(
        &file(vec![set(
            "s",
            &["n1", "no_such"],
            vec![tier(2, vec![rerolls(1)])],
        )]),
        at,
    );
    // An unnamed reward is not a star of a set.
    invalid_at(
        &file(vec![set(
            "s",
            &["n1", "a"],
            vec![tier(2, vec![rerolls(1)])],
        )]),
        at,
    );
}

#[test]
fn a_set_has_two_to_four_members() {
    let members = "$.constellations[0].members";
    invalid_at(
        &file(vec![set("s", &["n1"], vec![tier(1, vec![rerolls(1)])])]),
        members,
    );
    invalid_at(
        &file(vec![set(
            "s",
            &["n1", "n2", "n3", "n4", "n5"],
            vec![tier(5, vec![rerolls(1)])],
        )]),
        members,
    );
    parse(&file(vec![set(
        "s",
        &["n1", "n2", "n3", "n4"],
        vec![tier(4, vec![rerolls(1)])],
    )]))
    .expect("four is the most");
    parse(&file(vec![set(
        "s",
        &["n1", "n2"],
        vec![tier(2, vec![rerolls(1)])],
    )]))
    .expect("two is the fewest");
}

#[test]
fn a_named_star_is_in_at_most_one_set() {
    let two = |second: &[&str]| {
        file(vec![
            set("s", &["n1", "n2"], vec![tier(2, vec![rerolls(1)])]),
            set("t", second, vec![tier(2, vec![rerolls(1)])]),
        ])
    };
    parse(&two(&["n3", "n4"])).expect("disjoint sets");
    invalid_at(&two(&["n2", "n4"]), "$.constellations[1].members[0]");
    // Nor twice in one set.
    invalid_at(
        &file(vec![set(
            "s",
            &["n1", "n1"],
            vec![tier(2, vec![rerolls(1)])],
        )]),
        "$.constellations[0].members[1]",
    );
}

#[test]
fn ids_are_well_formed_and_unique_and_names_are_bounded() {
    let one = |id: &str| set(id, &["n1", "n2"], vec![tier(2, vec![rerolls(1)])]);
    invalid_at(&file(vec![one("Bad Id")]), "$.constellations[0].id");
    let twice = file(vec![
        one("s"),
        set("s", &["n3", "n4"], vec![tier(2, vec![rerolls(1)])]),
    ]);
    invalid_at(&twice, "$.constellations[1].id");
    let mut s = one("s");
    s["name"] = json!("");
    invalid_at(&file(vec![s]), "$.constellations[0].name");
    let mut s = one("s");
    s["name"] = json!("x".repeat(33));
    invalid_at(&file(vec![s]), "$.constellations[0].name");
}

#[test]
fn tiers_ascend_and_the_top_one_is_the_whole_set() {
    let of = |tiers: Vec<Value>| file(vec![set("s", &["n1", "n2", "n3"], tiers)]);
    let e = |n| vec![rerolls(n)];
    invalid_at(&of(vec![]), "$.constellations[0].tiers");
    // The top tier is the member count.
    invalid_at(&of(vec![tier(2, e(1))]), "$.constellations[0].tiers");
    // Strictly ascending.
    invalid_at(
        &of(vec![tier(2, e(1)), tier(2, e(2)), tier(3, e(3))]),
        "$.constellations[0].tiers[1].threshold",
    );
    invalid_at(
        &of(vec![tier(3, e(1)), tier(2, e(2))]),
        "$.constellations[0].tiers[1].threshold",
    );
    // From 1 to the member count.
    invalid_at(
        &of(vec![tier(0, e(1)), tier(3, e(2))]),
        "$.constellations[0].tiers[0].threshold",
    );
    invalid_at(
        &of(vec![tier(4, e(1))]),
        "$.constellations[0].tiers[0].threshold",
    );
    parse(&of(vec![tier(3, e(1))])).expect("one tier, the whole set");
    parse(&of(vec![tier(1, e(1)), tier(2, e(2)), tier(3, e(3))])).expect("a tier at each count");
}

#[test]
fn a_tiers_effects_are_the_reward_effects_with_the_same_caps() {
    let of = |effects: Vec<Value>| file(vec![set("s", &["n1", "n2"], vec![tier(2, effects)])]);
    invalid_at(&of(vec![]), "$.constellations[0].tiers[0].effects");
    invalid_at(
        &of(vec![
            rerolls(1),
            rerolls(1),
            rerolls(1),
            rerolls(1),
            rerolls(1),
        ]),
        "$.constellations[0].tiers[0].effects",
    );
    parse(&of(vec![rerolls(1), rerolls(1), rerolls(1), rerolls(1)])).expect("four effects");
    // One entry beyond its kind's largest amount.
    invalid_at(
        &of(vec![rerolls(11)]),
        "$.constellations[0].tiers[0].effects[0].amount",
    );
    // A tier's entries add up within the kind's largest, as a reward's do.
    invalid_at(
        &of(vec![rerolls(6), rerolls(5)]),
        "$.constellations[0].tiers[0].effects",
    );
    parse(&of(vec![rerolls(5), max_hp(1_000)])).expect("two kinds");
    // A random rune keys on a reward, not on a set.
    invalid_at(
        &of(vec![json!({ "effect": "random_rune" })]),
        "$.constellations[0].tiers[0].effects[0].effect",
    );
}

#[test]
fn the_full_tier_holds_at_least_what_the_small_one_does() {
    // The engine applies the difference between tiers, so a higher tier may not hold less.
    let of = |low: Vec<Value>, high: Vec<Value>| {
        file(vec![set(
            "s",
            &["n1", "n2", "n3"],
            vec![tier(2, low), tier(3, high)],
        )])
    };
    parse(&of(vec![rerolls(2)], vec![rerolls(2)])).expect("the same");
    parse(&of(vec![rerolls(2)], vec![rerolls(3), max_hp(10)])).expect("more, and another kind");
    invalid_at(
        &of(vec![rerolls(3)], vec![rerolls(2)]),
        "$.constellations[0].tiers[1].effects",
    );
    invalid_at(
        &of(vec![rerolls(3), max_hp(10)], vec![rerolls(3)]),
        "$.constellations[0].tiers[1].effects",
    );
}

#[test]
fn at_most_sixteen_sets() {
    assert_eq!(MAX_CONSTELLATIONS, 16);
    // Two members each leaves all 32 named stars of a file in sets, the most a file holds.
    let mut list: Vec<Value> = (0..32)
        .map(|i| named(&format!("star{i}"), &format!("Star {i}")))
        .collect();
    list.push(plain("a", 1));
    let rewards = rewards_of(list);
    let sets = |n: usize| {
        file(
            (0..n)
                .map(|i| {
                    set(
                        &format!("set{i}"),
                        &[
                            &format!("star{}", 2 * (i % 16)),
                            &format!("star{}", 2 * (i % 16) + 1),
                        ],
                        vec![tier(2, vec![rerolls(1)])],
                    )
                })
                .collect(),
        )
    };
    let parse = |v: &Value| {
        Constellations::parse_with(v.to_string().as_bytes(), Versions::CURRENT, &rewards)
    };
    assert_eq!(parse(&sets(16)).expect("sixteen").sets().len(), 16);
    match parse(&sets(17)) {
        Err(Error::Invalid(issues)) => assert!(issues.iter().any(|i| i.path == "$.constellations")),
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[test]
fn the_file_is_read_as_untrusted_bytes() {
    let rewards = rewards_of(reward_list());
    let big = vec![b' '; MAX_CONSTELLATIONS_BYTES + 1];
    assert!(matches!(
        Constellations::parse_with(&big, Versions::CURRENT, &rewards),
        Err(Error::TooLarge { .. })
    ));
    let mut v = file(vec![trio()]);
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file(vec![trio()]);
    v["constellations"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file(vec![trio()]);
    v["schema_version"] = json!(0);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let v = file(vec![trio()]);
    assert!(matches!(
        parse_at(
            &v,
            Versions {
                schema_version: 99,
                sim_version: 0
            }
        ),
        Err(Error::UnsupportedVersion { .. })
    ));
    assert!(matches!(
        Constellations::parse_with(b"not json", Versions::CURRENT, &rewards),
        Err(Error::Parse { .. })
    ));
}

// --- The pack ----------------------------------------------------------------------------

/// The bundled pack with the named stars and a set added, as a value.
fn pack_value(sets: Option<Value>) -> Value {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    // The bundled pack holds the real sets: a pack as released before them has neither the file
    // nor the list of its ids.
    v.as_object_mut().unwrap().remove("constellations");
    v["introduced"]
        .as_object_mut()
        .unwrap()
        .remove("constellations");
    for id in ["n1", "n2"] {
        v["rewards"]["rewards"]
            .as_array_mut()
            .unwrap()
            .push(named(id, id));
        v["introduced"]["rewards"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "id": id, "sim_version": SIM_VERSION }));
    }
    if let Some(sets) = sets {
        v["constellations"] = sets;
    }
    v
}

fn pair() -> Value {
    set("pair", &["n1", "n2"], vec![tier(2, vec![rerolls(1)])])
}

fn load(v: &Value) -> Result<Pack, PackError> {
    Pack::from_bytes(&serde_json::to_vec(v).unwrap())
}

#[test]
fn a_pack_may_hold_the_file_and_one_released_without_it_has_no_sets() {
    // New content at sim_version 2 may not hold an offer effect, so the set gives max_hp.
    let pair = set("pair", &["n1", "n2"], vec![tier(2, vec![max_hp(1)])]);
    let with = load(&pack_value(Some(file(vec![pair])))).expect("a pack with sets");
    assert_eq!(with.content().constellations().sets().len(), 1);
    assert_eq!(with.content().rewards().constellations().sets().len(), 1);
    let without = load(&pack_value(None)).expect("a pack released before the file");
    assert!(without.content().constellations().is_empty());
    assert_eq!(Pack::bundled().content().constellations().sets().len(), 4);
    // Not the same pack: the file is part of its hash.
    assert_ne!(with.sha256(), without.sha256());
}

#[test]
fn a_set_is_checked_against_the_packs_own_rewards() {
    let bad = set("pair", &["n1", "n9"], vec![tier(2, vec![rerolls(1)])]);
    match load(&pack_value(Some(file(vec![bad])))) {
        Err(PackError::File {
            file: "constellations",
            error: Error::Invalid(_),
        }) => {}
        other => panic!("expected a refused constellations file, got {other:?}"),
    }
}

#[test]
fn the_pack_counts_the_sets_before_reading_them() {
    let many = file(vec![pair(); MAX_CONSTELLATIONS + 1]);
    match load(&pack_value(Some(many))) {
        Err(PackError::Budget { path, limit, found }) => {
            assert_eq!(path, "$.constellations.constellations");
            assert_eq!((limit, found), (MAX_CONSTELLATIONS, MAX_CONSTELLATIONS + 1));
        }
        other => panic!("expected a budget error, got {other:?}"),
    }
}

// --- In a run ----------------------------------------------------------------------------

/// The trio's three stars and four commons (offers hold three).
fn trio_list() -> Vec<Value> {
    let mut list = reward_list();
    list.retain(|r| !["n4", "n5"].contains(&r["id"].as_str().unwrap()));
    list
}

/// An orbit of `amount` orbs for a set's tier: each orb one active phase.
fn orbit_effect(amount: u32) -> Value {
    json!({
        "effect": "orbit", "amount": amount,
        "orbit": {
            "attack": { "inline": [
                { "kind": "active", "ticks": 4, "damage": 7,
                  "hit_box": { "type": "circle", "offset": { "forward": 10_240 },
                               "radius": 1_024 } }
            ] },
            "turn_per_tick": 3_840,
            "cooldown_ticks": 60
        }
    })
}

fn orbit_trio() -> Value {
    set(
        "trio",
        &["n1", "n2", "n3"],
        vec![
            tier(2, vec![rerolls(1)]),
            tier(3, vec![rerolls(1), orbit_effect(3)]),
        ],
    )
}

#[test]
fn a_tier_may_give_an_orbit_but_not_a_random_rune() {
    parse(&file(vec![orbit_trio()])).expect("a tier with an orbit validates");
    let mut bare = orbit_trio();
    bare["tiers"][1]["effects"][1]["orbit"] = Value::Null;
    invalid_at(
        &file(vec![bare]),
        "$.constellations[0].tiers[1].effects[1].orbit",
    );
    let rune = set(
        "trio",
        &["n1", "n2", "n3"],
        vec![
            tier(2, vec![rerolls(1)]),
            tier(3, vec![rerolls(1), json!({ "effect": "random_rune" })]),
        ],
    );
    invalid_at(
        &file(vec![rune]),
        "$.constellations[0].tiers[1].effects[1].effect",
    );
}

/// A run on a bare stage with the trio's members equipped at Lv1: `n1` attack (slot 0), `n2`
/// defence (slot 5), `n3` mobility (slot 7), given distinct effects so they can be held
/// together; `held` of them, in that order (ADR-SIM-7). `first` replaces `n1`'s effects.
fn trio_equipped(held: usize, first: Value, sets: Value) -> Engine {
    let mut list = trio_list();
    list[0]["effects"] = json!([first]);
    list[1]["effects"] = json!([max_hp(1)]);
    list[2]["effects"] = json!([{ "effect": "speed_pct", "amount": 1 }]);
    let file = json!({ "rarities": crate::common::rarities(), "rewards": list });
    let parts = crate::common::equip::Parts {
        sets: Some(sets),
        ..Default::default()
    };
    let stars = [("n1", 0), ("n2", 5), ("n3", 7)];
    let stage = near(1);
    crate::common::equip::equipped_in(&stage, file, parts, &stars[..held], 1)
}

#[test]
fn reaching_a_tier_with_an_orbit_gives_its_orbs() {
    let first = json!({ "effect": "attack_pct", "amount": 1 });
    for held in 0..3 {
        let e = trio_equipped(held, first.clone(), json!([orbit_trio()]));
        assert!(e.snapshot().players[0].orbits.is_empty(), "{held} held");
    }
    let e = trio_equipped(3, first, json!([orbit_trio()]));
    let snapshot = e.snapshot();
    let orbits = &snapshot.players[0].orbits;
    assert_eq!(orbits.len(), 1);
    assert_eq!((orbits[0].reward.as_str(), orbits[0].orbs), ("trio", 3));
}

#[test]
fn a_tier_that_adds_burst_depth_applies_once() {
    let burst = json!({ "effect": "kill_burst", "amount": 10, "radius": 50, "depth": 3 });
    let trio = set(
        "trio",
        &["n1", "n2", "n3"],
        vec![
            tier(2, vec![rerolls(1)]),
            tier(
                3,
                vec![rerolls(1), json!({ "effect": "burst_depth", "amount": 2 })],
            ),
        ],
    );
    let mut e = trio_equipped(3, burst, json!([trio]));
    for _ in 0..30 {
        e.step(&[
            crate::common::idle(),
            Default::default(),
            Default::default(),
        ]);
    }
    assert_eq!(e.snapshot().players[0].burst_depth, 5);
}

#[test]
fn a_sets_tiers_give_one_orbit_and_orbits_are_counted_with_the_rewards() {
    // The same orbit at both tiers, 1 orb then 3.
    let both = set(
        "trio",
        &["n1", "n2", "n3"],
        vec![
            tier(2, vec![orbit_effect(1)]),
            tier(3, vec![orbit_effect(3)]),
        ],
    );
    parse(&file(vec![both])).expect("the same orbit with more orbs");
    // Another orbit at the top tier is refused.
    let mut other = orbit_effect(3);
    other["orbit"]["cooldown_ticks"] = json!(90);
    let differs = set(
        "trio",
        &["n1", "n2", "n3"],
        vec![tier(2, vec![orbit_effect(1)]), tier(3, vec![other])],
    );
    invalid_at(&file(vec![differs]), "$.constellations[0].tiers");
}
