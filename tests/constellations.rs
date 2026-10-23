//! The constellations (core#NS3, app#336): sets of named stars with tiers. The file is read as
//! untrusted bytes (ADR-020): members must be named stars of the run's rewards, each in at most
//! one set, 2 to 4 to a set; the tiers ascend and the top one is the whole set; a tier's
//! effects are the reward effects, held to the same caps. A tier lists the effects the set
//! has **at that tier**: reaching the full tier replaces the small one's, it is not added to
//! it. The engine counts the members held after every take, carried stars included.

use serde_json::{Value, json};
use stella_rain_core::constellations::{
    Constellations, MAX_CONSTELLATIONS, MAX_CONSTELLATIONS_BYTES,
};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{CarriedStar, RunSetup};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::ConstellationProgress;
use stella_rain_core::stage::{RarityWeights, Stage, Tier};
use stella_rain_core::validate::Error;

use crate::common::run::{plain_content, plain_setup};
use crate::reward_kinds::{near, next_offer, offer, plain, take};

fn named(id: &str, star: &str) -> Value {
    json!({ "id": id, "rarity": "epic", "named": true, "star": star, "limit": 1,
            "effects": [{ "effect": "attack_pct", "amount": 1 }],
            "title": star, "description": "" })
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
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": list });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
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
    json!({ "schema_version": 0, "sim_version": 0, "constellations": sets })
}

fn parse(v: &Value) -> Result<Constellations, Error> {
    Constellations::parse_with(v.to_string().as_bytes(), &rewards_of(reward_list()))
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
    assert_eq!(c.sets()[0].id.0, "trio");
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
    let parse = |v: &Value| Constellations::parse_with(v.to_string().as_bytes(), &rewards);
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
        Constellations::parse_with(&big, &rewards),
        Err(Error::TooLarge { .. })
    ));
    let mut v = file(vec![trio()]);
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file(vec![trio()]);
    v["constellations"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file(vec![trio()]);
    v["schema_version"] = json!(99);
    assert!(matches!(parse(&v), Err(Error::UnsupportedVersion { .. })));
    assert!(matches!(
        Constellations::parse_with(b"not json", &rewards),
        Err(Error::Parse { .. })
    ));
}

// --- The pack ----------------------------------------------------------------------------

/// The bundled pack with the named stars and a set added, as a value.
fn pack_value(sets: Option<Value>) -> Value {
    let mut v: Value = serde_json::from_slice(&stella_rain_core::pack::export_bundled()).unwrap();
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
            .push(json!({ "id": id, "sim_version": 0 }));
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
    let with = load(&pack_value(Some(file(vec![pair()])))).expect("a pack with sets");
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

/// Stars every kill drops as epic, and enemies enough for a dozen offers.
fn epic_near() -> Stage {
    let mut stage = near(14);
    let stars = stage.stars.as_mut().expect("stars");
    stars.tiers = vec![Tier {
        from_level: 1,
        weights: RarityWeights {
            epic: 1,
            ..RarityWeights::default()
        },
        random_rune_weight: None,
    }];
    stage
}

/// The trio's three stars and four commons (offers hold three).
fn trio_list() -> Vec<Value> {
    let mut list = reward_list();
    list.retain(|r| !["n4", "n5"].contains(&r["id"].as_str().unwrap()));
    list
}

fn engine(stage: &Stage, list: Vec<Value>, sets: Option<Value>) -> Engine {
    let rewards = rewards_of(list);
    let rewards = match sets {
        Some(v) => {
            let c = Constellations::parse_with(v.to_string().as_bytes(), &rewards)
                .expect("the test sets validate");
            rewards.with_constellations(c)
        }
        None => rewards,
    };
    Engine::new_with(
        stage,
        &RunContent::new(RunStats::bundled().clone(), rewards),
    )
    .expect("the test stage is valid")
}

fn tiers_reached(e: &Engine) -> Vec<(String, u8)> {
    e.events()
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::Constellation { set, tier } => Some((set.0.clone(), *tier)),
            _ => None,
        })
        .collect()
}

fn star_taken(e: &Engine) -> Option<String> {
    e.events().iter().find_map(|ev| match ev {
        DomainEvent::RewardTaken { star, .. } => star.clone(),
        _ => None,
    })
}

#[test]
fn the_small_tier_applies_at_two_of_three_and_the_full_one_replaces_it() {
    let mut stage = epic_near();
    // A seed whose first offer holds Alpha (n1); the offers are drawn.
    stage.seed = 1;
    let mut e = engine(&stage, trio_list(), Some(file(vec![trio()])));
    next_offer(&mut e);
    assert_eq!(offer(&e).rerolls_left, 0);
    // A member shows its set and what is held and needed; a common shows none.
    let o = offer(&e);
    let progress = |reward: &str| {
        o.candidates
            .iter()
            .find(|c| c.reward.0 == reward)
            .unwrap_or_else(|| panic!("{reward} is not offered"))
            .constellation
            .clone()
    };
    assert_eq!(
        progress("n1"),
        Some(ConstellationProgress {
            set: "trio".into(),
            held: 0,
            needed: 2
        })
    );
    take(&mut e, "n1");
    assert_eq!(star_taken(&e).as_deref(), Some("Alpha"));
    assert!(tiers_reached(&e).is_empty(), "one of three is no tier");
    let view = &e.snapshot().constellations;
    assert_eq!(
        (view[0].id.0.as_str(), view[0].held, view[0].tier),
        ("trio", 1, 0)
    );

    next_offer(&mut e);
    assert_eq!(offer(&e).rerolls_left, 0);
    take(&mut e, "n2");
    assert_eq!(tiers_reached(&e), [("trio".to_string(), 1)]);
    assert_eq!(e.snapshot().constellations[0].tier, 1);

    next_offer(&mut e);
    assert_eq!(offer(&e).rerolls_left, 1, "the small tier's reroll");
    let o = offer(&e);
    let n3 = o.candidates.iter().find(|c| c.reward.0 == "n3").unwrap();
    assert_eq!(
        n3.constellation,
        Some(ConstellationProgress {
            set: "trio".into(),
            held: 2,
            needed: 3
        })
    );
    take(&mut e, "n3");
    assert_eq!(tiers_reached(&e), [("trio".to_string(), 2)]);

    next_offer(&mut e);
    // The full tier has 3 rerolls in all: 3, not 1 + 3.
    assert_eq!(offer(&e).rerolls_left, 3);
    let view = &e.snapshot().constellations;
    assert_eq!((view[0].held, view[0].tier), (3, 2));
    // A pick that is no member changes nothing, and no tier is reached twice.
    take(&mut e, "a");
    assert!(tiers_reached(&e).is_empty());
    next_offer(&mut e);
    assert_eq!(offer(&e).rerolls_left, 3);
}

#[test]
fn a_tier_that_lists_two_kinds_applies_the_difference_of_each() {
    let two = set(
        "trio",
        &["n1", "n2", "n3"],
        vec![
            tier(2, vec![rerolls(1)]),
            tier(3, vec![rerolls(2), max_hp(1_000)]),
        ],
    );
    let mut e = engine(&epic_near(), trio_list(), Some(file(vec![two])));
    for star in ["n1", "n2", "n3"] {
        next_offer(&mut e);
        take(&mut e, star);
    }
    next_offer(&mut e);
    assert_eq!(offer(&e).rerolls_left, 2);
}

#[test]
fn a_star_outside_every_set_has_no_progress_and_changes_no_set() {
    let pair = set("pair", &["n1", "n2"], vec![tier(2, vec![rerolls(1)])]);
    let mut e = engine(&epic_near(), trio_list(), Some(file(vec![pair])));
    next_offer(&mut e);
    let o = offer(&e);
    assert!(
        o.candidates
            .iter()
            .filter(|c| c.reward.0 == "n3" || c.reward.0 == "a")
            .all(|c| c.constellation.is_none())
    );
    take(&mut e, "n3");
    assert_eq!(e.snapshot().constellations[0].held, 0);
}

#[test]
fn a_run_without_sets_keeps_its_hashes() {
    // The same run with no file, with a set none of its picks touches, and with one they do.
    let hashes = |sets: Option<Value>| -> Vec<u64> {
        let mut e = engine(&epic_near(), trio_list(), sets);
        next_offer(&mut e);
        let mut out = vec![e.state_hash()];
        take(&mut e, "n1");
        out.push(e.state_hash());
        next_offer(&mut e);
        out.push(e.state_hash());
        out
    };
    let none = hashes(None);
    let empty = hashes(Some(file(vec![])));
    let untouched = hashes(Some(file(vec![set(
        "pair",
        &["n2", "n3"],
        vec![tier(2, vec![rerolls(1)])],
    )])));
    assert_eq!(none, empty);
    assert_eq!(none, untouched, "counts hash only while above 0");
    let touched = hashes(Some(file(vec![trio()])));
    assert_eq!(none[0], touched[0]);
    assert_ne!(none[1], touched[1], "one member held is part of the state");
}

#[test]
fn carried_stars_count_at_the_start_and_the_full_tier_replaces_the_small() {
    let hp_set = set(
        "pair",
        &["n1", "n2"],
        vec![tier(1, vec![max_hp(1_000)]), tier(2, vec![max_hp(3_000)])],
    );
    let rewards = rewards_of(reward_list());
    let sets = Constellations::parse_with(file(vec![hp_set]).to_string().as_bytes(), &rewards)
        .expect("valid");
    let content = RunContent::new(
        RunStats::bundled().clone(),
        rewards.with_constellations(sets),
    )
    .with_perk_sets(plain_content().perk_sets().clone());
    let start = |carried: &[&str]| {
        let setup = RunSetup {
            carried: carried
                .iter()
                .map(|id| CarriedStar {
                    reward: (*id).into(),
                })
                .collect(),
            lumine: 8,
            ..plain_setup()
        };
        Engine::start_with(&setup, &content).expect("a valid run")
    };
    let base = start(&[]).snapshot().player.hp;
    let one = start(&["n1"]);
    assert_eq!(one.snapshot().player.hp, base + 1_000);
    assert_eq!(one.snapshot().constellations[0].held, 1);
    let both = start(&["n1", "n2"]);
    // 3,000 in all, not 1,000 and 3,000.
    assert_eq!(both.snapshot().player.hp, base + 3_000);
    let view = &both.snapshot().constellations[0];
    assert_eq!((view.held, view.tier), (2, 2));
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

#[test]
fn reaching_a_tier_with_an_orbit_gives_its_orbs() {
    let mut e = engine(&epic_near(), trio_list(), Some(file(vec![orbit_trio()])));
    for star in ["n1", "n2", "n3"] {
        assert!(e.snapshot().player.orbits.is_empty(), "before {star}");
        next_offer(&mut e);
        take(&mut e, star);
    }
    let orbits = &e.snapshot().player.orbits;
    assert_eq!(orbits.len(), 1);
    assert_eq!((orbits[0].reward.0.as_str(), orbits[0].orbs), ("trio", 3));
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
    let mut list = trio_list();
    list[0]["effects"] = json!([burst]);
    let mut e = engine(&epic_near(), list, Some(file(vec![trio])));
    for star in ["n1", "n2", "n3"] {
        next_offer(&mut e);
        take(&mut e, star);
    }
    assert_eq!(e.snapshot().player.burst_depth, 5);
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
