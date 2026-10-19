//! The expertise offers (ADR-078 §4, §6; ADR-079 §2, §3): each level reached opens one offer
//! at the end of the tick, for the weapon ADR-078's table gives, in level order. Common levels
//! (1, 4, 7 Main; 2, 8 Sub; 11 and up Main, Sub, Main) draw from that weapon's common pool.
//! Level 3 (Main) and 5 (Sub) show that weapon's three styles; 6 and 9 the chosen Main style's
//! upgrades, 10 its capstone: fixed choices, no draw and no reroll. A common offer has a
//! reroll of each candidate; no expertise offer has a skip or a banish. Attack speed options have a stack limit: a candidate whose
//! pick reaches it is marked, and an option at it leaves the pool. On one tick the boss offer
//! opens first, then the expertise offers in level order, then the stars'.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::input::Input;
use stella_rain_core::replay::hash_to_hex;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, record_run, verify_run};
use stella_rain_core::snapshot::{
    EntityKind, ExpertiseOfferKind, ExpertiseOfferView, ExpertiseView, OfferKind, OfferOption,
    WeaponRole,
};
use stella_rain_core::stage::Stage;

use crate::common;

use common::*;

/// 32 pixels above it: inside the `blade`'s slash and the pickup circle.
const NEAR_Y: i32 = START_Y - 8_192;

/// Level 1 needs 10 points, level 2 20 more, and so on to 100 for level 10; 150, 225 and 337
/// for levels 11, 12 and 13.
const TABLE: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// The points that take the gauge from 0 to level `n`.
fn to_level(n: usize) -> u32 {
    let mut total: u32 = TABLE.iter().take(n).sum();
    let mut need = TABLE[9];
    for _ in 10..n {
        need = need * 150 / 100;
        total += need;
    }
    total
}

/// A common option `id` of `effect` and `amount`, with `limit` when it has one.
fn option(id: &str, effect: &str, amount: u32, limit: Option<u8>) -> Value {
    let mut v = json!({ "id": id, "effect": effect, "amount": amount,
                        "title": "Option", "description": "" });
    if let Some(l) = limit {
        v["limit"] = json!(l);
    }
    v
}

/// The damage options `prefix0`, `prefix1`, ...: `n` of them, 10 percent each.
fn damages(prefix: &str, n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| option(&format!("{prefix}{i}"), "attack_pct", 10, None))
        .collect()
}

/// The bundled content, its expertise with `TABLE` (growth 150 percent, at most 100_000), and
/// the `blade`'s and the `maul`'s common pools replaced by `main` and `sub`.
fn content(main: Vec<Value>, sub: Vec<Value>) -> RunContent {
    content_with(TABLE, 150, 100_000, main, sub)
}

fn content_with(
    thresholds: [u32; 10],
    growth_pct: u16,
    max_points: u32,
    main: Vec<Value>,
    sub: Vec<Value>,
) -> RunContent {
    parsed(file_with(thresholds, growth_pct, max_points, main, sub))
}

/// The bundled expertise file with `thresholds`, `growth_pct`, `max_points`, and the `blade`'s
/// and the `maul`'s common pools replaced by `main` and `sub`.
fn file_with(
    thresholds: [u32; 10],
    growth_pct: u16,
    max_points: u32,
    main: Vec<Value>,
    sub: Vec<Value>,
) -> Value {
    let mut file: Value = serde_json::from_slice(Expertise::BUNDLED).unwrap();
    file["thresholds"] = json!(thresholds);
    file["growth_pct"] = json!(growth_pct);
    file["max_points"] = json!(max_points);
    for w in file["weapons"].as_array_mut().unwrap() {
        let pool = match w["weapon"].as_str().unwrap() {
            "blade" => main.clone(),
            "maul" => sub.clone(),
            other => panic!("no pool for {other}"),
        };
        w["common"] = json!(pool);
    }
    file
}

fn parsed(file: Value) -> RunContent {
    let expertise =
        Expertise::parse(file.to_string().as_bytes()).expect("the test expertise validates");
    RunContent::bundled().clone().with_expertise(expertise)
}

/// An enemy of `hp` that never attacks, holding at `(x, y)`, giving `kill` points for its kill.
fn target(hp: u32, x: i32, y: i32, kill: u32) -> Value {
    let mut w = wave(1, 1, 0, enemy(hp, hold(x, y), json!([])), x, y);
    w["expertise"] = json!({ "hit": 0, "kill": kill });
    w
}

/// The `blade` as Main and the `maul` as Sub, levels of 1000 experience, stars that every kill
/// drops (taken at once within 48 pixels), one reroll and one banish; a boss of `boss_hp` 32
/// pixels ahead giving `boss_kill` points, or none; and `waves`.
fn stage(boss: Option<(u32, u32)>, waves: Vec<Value>) -> Stage {
    let boss = boss.map(|(hp, kill)| {
        let mut b = boss_at(hp, NEAR_Y);
        b["expertise"] = json!({ "hit": 0, "kill": kill });
        b
    });
    let mut stage = stage_full(18_000, boss.or(Some(far_boss())), waves, vec![]);
    stage = armed(
        stage,
        json!({ "preset": { "id": "blade" } }),
        json!({ "preset": { "id": "maul" } }),
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 1, "banishes": 1,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

fn with(stage: &Stage, content: &RunContent) -> Engine {
    Engine::new_with(stage, content).expect("the test stage is valid")
}

fn gauge(e: &Engine) -> ExpertiseView {
    e.snapshot().expertise.expect("the stage has the gauge")
}

fn expertise_offer(e: &Engine) -> ExpertiseOfferView {
    gauge(e).offer.expect("an open expertise offer")
}

fn candidates(e: &Engine) -> Vec<String> {
    expertise_offer(e)
        .candidates
        .into_iter()
        .map(|c| c.option.0)
        .collect()
}

/// Presses `main` every 6 ticks until an offer opens, returning the events of the ticks run.
fn until_offer(e: &mut Engine) -> Vec<DomainEvent> {
    let seen = fire_until(e, Input::default(), 600, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 600 ticks");
    seen.into_iter().map(|(_, ev)| ev).collect()
}

/// What each offer opening in `events` is: `boss`, `star`, or the weapon and the level.
fn openings(events: &[DomainEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::OfferOpened { kind, .. } => Some(match kind {
                OfferKind::Boss => "boss".to_string(),
                OfferKind::Star => "star".to_string(),
            }),
            DomainEvent::ExpertiseOfferOpened { level, weapon, .. } => Some(format!(
                "{} {level}",
                match weapon {
                    WeaponRole::Main => "main",
                    WeaponRole::Sub => "sub",
                }
            )),
            _ => None,
        })
        .collect()
}

/// Takes candidate 0 of every offer as it opens, until none is open; returns the openings,
/// the first one included when it is in `first`.
fn take_all(e: &mut Engine, first: &[DomainEvent]) -> Vec<String> {
    let mut seen = openings(first);
    while e.offer_open() {
        e.choose(0).unwrap();
        seen.extend(openings(e.events()));
    }
    seen
}

fn index_of(e: &Engine, option: OfferOption) -> u32 {
    let options = expertise_offer(e).options;
    let i = options
        .iter()
        .position(|o| *o == option)
        .unwrap_or_else(|| panic!("{option:?} is not offered: {options:?}"));
    u32::try_from(i).unwrap()
}

/// The index of the candidate whose option is `id`.
fn take_index(e: &Engine, id: &str) -> u32 {
    let at = candidates(e)
        .iter()
        .position(|c| c == id)
        .unwrap_or_else(|| panic!("{id} is not a candidate: {:?}", candidates(e)));
    index_of(
        e,
        OfferOption::Take {
            candidate: u8::try_from(at).unwrap(),
        },
    )
}

#[test]
fn a_boss_kill_crossing_several_thresholds_opens_one_offer_per_level_in_order() {
    let mut e = with(
        &stage(Some((1, to_level(13))), vec![]),
        &content(damages("m", 3), damages("s", 3)),
    );
    let first = until_offer(&mut e);
    let seen = take_all(&mut e, &first);
    // The style path (3, 5, 6, 9, 10) opens in level order with the rest; 11 and up go Main,
    // Sub, Main.
    assert_eq!(
        seen,
        [
            "boss", "main 1", "sub 2", "main 3", "main 4", "sub 5", "main 6", "main 7", "sub 8",
            "main 9", "main 10", "main 11", "sub 12", "main 13"
        ]
    );
    let g = gauge(&e);
    assert_eq!((g.level, g.points), (13, 0));
    let sum = |t: &[u8]| t.iter().map(|n| u32::from(*n)).sum::<u32>();
    assert_eq!(sum(&g.main_taken), 5, "five Main offers, one pick each");
    assert_eq!(sum(&g.sub_taken), 3, "three Sub offers");
}

#[test]
fn an_expertise_offer_opens_at_the_end_of_the_tick_and_stops_the_world() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 3), damages("s", 3)),
    );
    let events = until_offer(&mut e);
    assert_eq!(
        openings(&events),
        ["main 1"],
        "the kill's tick opens level 1's offer"
    );
    let tick = e.tick();
    e.step(Input::default());
    assert_eq!(e.tick(), tick, "the world is stopped");
    assert!(e.events().is_empty());
}

#[test]
fn a_kill_s_level_opens_before_the_star_it_drops() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 3), damages("s", 3)),
    );
    let first = until_offer(&mut e);
    assert_eq!(take_all(&mut e, &first), ["main 1", "star"]);
}

#[test]
fn a_common_offer_holds_three_candidates_and_a_reroll_of_each_and_no_skip_or_banish() {
    use OfferOption::{Reroll, Take};
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 4), damages("s", 4)),
    );
    until_offer(&mut e);
    let offer = expertise_offer(&e);
    assert_eq!(
        (offer.level, offer.weapon, offer.kind),
        (1, WeaponRole::Main, ExpertiseOfferKind::Common)
    );
    assert_eq!(offer.candidates.len(), 3);
    assert_eq!(
        offer.options,
        [
            Take { candidate: 0 },
            Take { candidate: 1 },
            Take { candidate: 2 },
            Reroll { candidate: 0 },
            Reroll { candidate: 1 },
            Reroll { candidate: 2 },
        ],
        "a banish is left in the run, and neither it nor a skip is offered"
    );
    assert_eq!(offer.rerolls_left, 1);
}

#[test]
fn a_reroll_draws_that_candidate_alone_again_and_spends_a_reroll() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 4), damages("s", 4)),
    );
    until_offer(&mut e);
    let before = candidates(&e);
    let missing = ["m0", "m1", "m2", "m3"]
        .into_iter()
        .find(|id| !before.iter().any(|c| c == id))
        .unwrap();
    e.choose(index_of(&e, OfferOption::Reroll { candidate: 1 }))
        .unwrap();
    assert!(e.offer_open(), "a reroll leaves the offer open");
    let after = candidates(&e);
    assert_eq!(
        after,
        [before[0].as_str(), missing, before[2].as_str()],
        "only candidate 1 is drawn again, from what the offer does not hold"
    );
    assert_eq!(
        e.events(),
        [DomainEvent::ExpertiseCandidateReplaced {
            slot: 1,
            option: missing.into(),
        }]
    );
    let offer = expertise_offer(&e);
    assert_eq!(offer.rerolls_left, 0);
    assert_eq!(offer.options.len(), 3, "no reroll is left: only the takes");
}

#[test]
fn no_reroll_is_offered_when_no_other_option_could_come() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 3), damages("s", 3)),
    );
    until_offer(&mut e);
    assert_eq!(expertise_offer(&e).options.len(), 3);
}

#[test]
fn with_fewer_options_left_the_offer_holds_those() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 2), damages("s", 2)),
    );
    until_offer(&mut e);
    let mut c = candidates(&e);
    c.sort();
    assert_eq!(c, ["m0", "m1"]);
    assert_eq!(expertise_offer(&e).options.len(), 2);
}

#[test]
fn taking_an_option_applies_it_and_counts_it_for_its_weapon() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(vec![option("tough", "max_hp", 5, None)], damages("s", 1)),
    );
    until_offer(&mut e);
    let hp = e.snapshot().player.hp;
    e.choose(0).unwrap();
    assert_eq!(
        e.events().first(),
        Some(&DomainEvent::ExpertiseOptionTaken {
            weapon: WeaponRole::Main,
            option: "tough".into(),
            taken: 1,
        })
    );
    assert_eq!(e.snapshot().player.hp, hp + 5, "five more hit points");
    assert_eq!(gauge(&e).main_taken, [1]);
    assert_eq!(gauge(&e).sub_taken, [0]);
}

#[test]
fn an_attack_speed_option_reaching_its_limit_is_marked_then_leaves_the_pool() {
    let main = vec![
        option("quick", "attack_speed_pct", 10, Some(2)),
        option("hard", "attack_pct", 10, None),
    ];
    let mut e = with(
        &stage(Some((1, to_level(7))), vec![]),
        &content(main, damages("s", 1)),
    );
    until_offer(&mut e);
    e.choose(0).unwrap(); // The boss offer.
    let mut main_offers = Vec::new();
    while e.offer_open() {
        let offer = expertise_offer(&e);
        if offer.weapon == WeaponRole::Sub {
            e.choose(0).unwrap();
            continue;
        }
        let marked: Vec<(String, bool)> = offer
            .candidates
            .iter()
            .map(|c| (c.option.0.clone(), c.reaches_limit))
            .collect();
        main_offers.push((offer.level, marked));
        let pick = if candidates(&e).iter().any(|c| c == "quick") {
            take_index(&e, "quick")
        } else {
            0
        };
        e.choose(pick).unwrap();
    }
    let has = |level: u32, id: &str, mark: bool| {
        main_offers
            .iter()
            .any(|(l, c)| *l == level && c.contains(&(id.to_string(), mark)))
    };
    assert!(has(1, "quick", false), "{main_offers:?}");
    assert!(
        has(4, "quick", true),
        "the pick reaches its limit: {main_offers:?}"
    );
    assert_eq!(
        main_offers.last(),
        Some(&(7, vec![("hard".to_string(), false)])),
        "at its limit, it leaves the pool"
    );
    assert_eq!(gauge(&e).main_taken, [2, 1]);
}

#[test]
fn a_weapon_with_nothing_left_to_offer_opens_none_and_the_other_still_does() {
    let mut e = with(
        &stage(Some((1, to_level(4))), vec![]),
        &content(
            vec![option("quick", "attack_speed_pct", 10, Some(1))],
            damages("s", 1),
        ),
    );
    let first = until_offer(&mut e);
    assert_eq!(
        take_all(&mut e, &first),
        ["boss", "main 1", "sub 2", "main 3"],
        "level 4's Main offer has nothing to hold and does not open; level 3's styles do"
    );
    assert_eq!(gauge(&e).level, 4);
}

#[test]
fn a_boss_kill_no_longer_upgrades_a_weapon_its_points_are_its_weapon_progress() {
    let mut e = with(
        &stage(Some((1, to_level(1))), vec![]),
        &content(damages("m", 3), damages("s", 3)),
    );
    let events = until_offer(&mut e);
    assert!(
        !events.iter().any(|ev| matches!(
            ev,
            DomainEvent::RewardTaken { .. } | DomainEvent::ExpertiseOptionTaken { .. }
        )),
        "nothing is taken before a choice: {events:?}"
    );
    let s = e.snapshot();
    let g = s.expertise.unwrap();
    assert_eq!(g.level, 1, "the kill's points are its weapon progress");
    assert_eq!(g.main_taken, [0, 0, 0]);
    assert_eq!(g.sub_taken, [0, 0, 0]);
    assert_eq!(take_all(&mut e, &events), ["boss", "main 1"]);
}

#[test]
fn a_boss_kill_s_clear_opens_its_offer_then_its_levels_then_the_star_s() {
    // One slash kills the boss and an enemy beside it, whose star is taken at once. The kill
    // clears the stage: the boss offer opens, then the levels it reached, then the star's.
    let mut e = with(
        &stage(
            Some((1, to_level(2))),
            vec![target(1, START_X + 2_048, NEAR_Y, 0)],
        ),
        &content(damages("m", 3), damages("s", 3)),
    );
    let first = until_offer(&mut e);
    assert!(first.contains(&DomainEvent::StageCleared));
    assert!(
        first
            .iter()
            .any(|ev| matches!(ev, DomainEvent::StarTaken { .. }))
    );
    assert_eq!(
        take_all(&mut e, &first),
        ["boss", "main 1", "sub 2", "star"]
    );
    assert_eq!(gauge(&e).level, 2);
}

#[test]
fn damage_stays_bounded_with_many_damage_options_stacked() {
    // Every level after the table needs 10 points: a kill's 100_000 points cross about 10_000
    // levels, each a pick of 100 percent more damage, which the arithmetic of ADR-056
    // holds without overflowing.
    let big = |p: &str| vec![option(p, "attack_pct", 100, None)];
    let content = content_with([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 101, 10, big("m"), big("s"));
    let tough = target(100_000, START_X + 2_048, NEAR_Y, 0);
    let killed = target(1, START_X, NEAR_Y, 100_000);
    let mut e = with(&stage(None, vec![killed, tough]), &content);
    let first = until_offer(&mut e);
    let tough = e
        .snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy)
        .map(|v| v.id)
        .expect("the tough enemy outlives the first slash");
    let hits_on = |events: &[DomainEvent]| -> Vec<u32> {
        events
            .iter()
            .filter_map(|ev| match ev {
                DomainEvent::Hit { target, damage, .. } if *target == tough => Some(*damage),
                _ => None,
            })
            .collect()
    };
    let before = hits_on(&first);
    assert!(!before.is_empty(), "the slash hits the tough enemy too");
    let mut picks = 0u32;
    while e.offer_open() {
        e.choose(0).unwrap();
        picks += 1;
    }
    assert!(picks > 9_000, "{picks} offers");
    let g = gauge(&e);
    assert_eq!(
        g.main_taken,
        [u8::MAX],
        "the count stops at its most; the picks go on"
    );
    let later = fire_until(&mut e, Input::default(), 60, |e| e.offer_open());
    let later: Vec<DomainEvent> = later.into_iter().map(|(_, ev)| ev).collect();
    let after = hits_on(&later);
    assert!(!after.is_empty(), "the slash hits again");
    assert!(
        u64::from(after[0]) >= u64::from(before[0]) * 1_000,
        "{} then {}",
        before[0],
        after[0]
    );
}

#[test]
fn the_offer_and_the_picks_are_in_the_state_hash() {
    let make = || {
        with(
            &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
            &content(damages("m", 4), damages("s", 4)),
        )
    };
    let (mut a, mut b) = (make(), make());
    until_offer(&mut a);
    until_offer(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    b.choose(index_of(&b, OfferOption::Reroll { candidate: 0 }))
        .unwrap();
    assert_ne!(a.state_hash(), b.state_hash(), "the candidates are hashed");
    let (mut c, mut d) = (make(), make());
    until_offer(&mut c);
    until_offer(&mut d);
    c.choose(0).unwrap();
    d.choose(1).unwrap();
    // Each takes 10 percent more damage, so only the pick's place tells them apart.
    assert_ne!(c.state_hash(), d.state_hash(), "the picks are hashed");
}

#[test]
fn a_run_with_expertise_choices_replays_and_verifies() {
    use common::run::{TRIAL_LENGTH, play_run, trial_setup};
    let commands = play_run(TRIAL_LENGTH);
    let mut e = Engine::start(&trial_setup()).unwrap();
    let (mut expertise_choices, mut style_choices) = (0, 0);
    for c in &commands {
        match c {
            Command::Step(input) => e.step(*input),
            Command::Choose(i) => {
                if let Some(offer) = e.snapshot().expertise.and_then(|g| g.offer) {
                    expertise_choices += 1;
                    if offer.kind != ExpertiseOfferKind::Common {
                        style_choices += 1;
                    }
                }
                e.choose(*i).unwrap();
            }
            Command::Spend(i) => e.spend(*i).unwrap(),
        }
    }
    assert!(
        expertise_choices >= 3,
        "the trial run answers expertise offers: {expertise_choices}"
    );
    assert!(
        style_choices >= 1,
        "the trial run chooses a style: {style_choices}"
    );
    assert!(e.snapshot().expertise.unwrap().main_style.is_some());
    let replay = record_run(&trial_setup(), &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(hash_to_hex(verified.final_hash), replay.final_hash);
}

/// Style `id`, which adds `amount` of `effect` when chosen; its upgrades at 6 and 9 and its
/// capstones at 10 are two choices each, `<id>_6a` and `<id>_6b` and so on, a hit point each.
fn style(id: &str, effect: &str, amount: u32) -> Value {
    let two = |level: u32| {
        vec![
            option(&format!("{id}_{level}a"), "max_hp", 1, None),
            option(&format!("{id}_{level}b"), "max_hp", 1, None),
        ]
    };
    json!({ "id": id, "effect": effect, "amount": amount, "title": "Style", "description": "",
            "upgrade_6": two(6), "upgrade_9": two(9), "capstone": two(10) })
}

/// Styles `<prefix>_a`, `<prefix>_b` and `<prefix>_c`, each 1 percent more damage.
fn styles(prefix: &str) -> [Value; 3] {
    ["a", "b", "c"].map(|s| style(&format!("{prefix}_{s}"), "attack_pct", 1))
}

/// `content` with three damage options per pool, the `blade`'s styles `main` and the `maul`'s
/// `sub`.
fn styled(main: [Value; 3], sub: [Value; 3]) -> RunContent {
    let mut file = file_with(TABLE, 150, 100_000, damages("m", 3), damages("s", 3));
    for w in file["weapons"].as_array_mut().unwrap() {
        w["styles"] = match w["weapon"].as_str().unwrap() {
            "blade" => json!(main),
            _ => json!(sub),
        };
    }
    parsed(file)
}

/// Takes candidate 0 of every open offer until the expertise offer of `level` is open,
/// returning the events of the last choice (of the ticks run, when it was already open).
fn next_offer_of(e: &mut Engine, level: u32, mut events: Vec<DomainEvent>) -> Vec<DomainEvent> {
    loop {
        if gauge(e).offer.is_some_and(|o| o.level == level) {
            return events;
        }
        assert!(e.offer_open(), "level {level}'s offer never opened");
        e.choose(0).unwrap();
        events = e.events().to_vec();
    }
}

/// Runs until an offer opens, then as `next_offer_of`.
fn to_offer_of(e: &mut Engine, level: u32) -> Vec<DomainEvent> {
    let events = until_offer(e);
    next_offer_of(e, level, events)
}

fn takes(n: u8) -> Vec<OfferOption> {
    (0..n)
        .map(|candidate| OfferOption::Take { candidate })
        .collect()
}

fn ids(list: &[&str]) -> Vec<stella_rain_core::id::ContentId> {
    list.iter().map(|id| (*id).into()).collect()
}

#[test]
fn level_3_offers_the_main_s_three_styles_fixed_with_no_reroll_skip_or_banish() {
    let mut e = with(
        &stage(Some((1, to_level(3))), vec![]),
        &styled(styles("main"), styles("sub")),
    );
    let events = to_offer_of(&mut e, 3);
    let offer = expertise_offer(&e);
    assert_eq!(
        (offer.weapon, offer.kind),
        (WeaponRole::Main, ExpertiseOfferKind::Style)
    );
    assert_eq!(
        candidates(&e),
        ["main_a", "main_b", "main_c"],
        "all three, in the content's order"
    );
    assert_eq!(
        offer.options,
        takes(3),
        "a reroll and a banish are left in the run, and none is offered"
    );
    assert_eq!(offer.rerolls_left, 1);
    assert!(
        offer.candidates.iter().all(|c| !c.reaches_limit),
        "a style has no stack limit"
    );
    assert!(
        events.contains(&DomainEvent::ExpertiseOfferOpened {
            level: 3,
            weapon: WeaponRole::Main,
            kind: ExpertiseOfferKind::Style,
            candidates: ids(&["main_a", "main_b", "main_c"]),
        }),
        "{events:?}"
    );
    let hash = e.state_hash();
    assert!(e.choose(3).is_err(), "no reroll after the candidates");
    assert_eq!(e.state_hash(), hash, "the refused choice changes nothing");
    assert_eq!(candidates(&e), ["main_a", "main_b", "main_c"]);
}

#[test]
fn level_5_offers_the_sub_s_three_styles() {
    let mut e = with(
        &stage(Some((1, to_level(5))), vec![]),
        &styled(styles("main"), styles("sub")),
    );
    let events = to_offer_of(&mut e, 5);
    let offer = expertise_offer(&e);
    assert_eq!(
        (offer.weapon, offer.kind),
        (WeaponRole::Sub, ExpertiseOfferKind::Style)
    );
    assert_eq!(candidates(&e), ["sub_a", "sub_b", "sub_c"]);
    assert_eq!(offer.options, takes(3));
    assert!(events.contains(&DomainEvent::ExpertiseOfferOpened {
        level: 5,
        weapon: WeaponRole::Sub,
        kind: ExpertiseOfferKind::Style,
        candidates: ids(&["sub_a", "sub_b", "sub_c"]),
    }));
}

#[test]
fn choosing_a_style_applies_its_effect_and_holds_it_for_the_run() {
    let main = [
        style("main_a", "max_hp", 5),
        style("main_b", "max_hp", 7),
        style("main_c", "max_hp", 9),
    ];
    let mut e = with(
        &stage(Some((1, to_level(5))), vec![]),
        &styled(main, styles("sub")),
    );
    to_offer_of(&mut e, 3);
    let hp = e.snapshot().player.hp;
    e.choose(1).unwrap();
    assert_eq!(
        e.events().first(),
        Some(&DomainEvent::ExpertiseOptionTaken {
            weapon: WeaponRole::Main,
            option: "main_b".into(),
            taken: 1,
        })
    );
    assert_eq!(e.snapshot().player.hp, hp + 7, "seven more hit points");
    let g = gauge(&e);
    assert_eq!(
        (g.main_style, g.sub_style),
        (Some("main_b".into()), None),
        "the Sub's style comes at level 5"
    );
    next_offer_of(&mut e, 5, Vec::new());
    e.choose(2).unwrap();
    let g = gauge(&e);
    assert_eq!(
        (g.main_style, g.sub_style),
        (Some("main_b".into()), Some("sub_c".into()))
    );
    assert!(g.main_path.is_empty());
}

#[test]
fn levels_6_and_9_offer_the_chosen_main_style_s_upgrades_and_10_its_capstone() {
    let mut e = with(
        &stage(Some((1, to_level(10))), vec![]),
        &styled(styles("main"), styles("sub")),
    );
    to_offer_of(&mut e, 3);
    e.choose(1).unwrap();
    for (level, kind, choices, pick) in [
        (
            6,
            ExpertiseOfferKind::Upgrade,
            ["main_b_6a", "main_b_6b"],
            1,
        ),
        (
            9,
            ExpertiseOfferKind::Upgrade,
            ["main_b_9a", "main_b_9b"],
            0,
        ),
        (
            10,
            ExpertiseOfferKind::Capstone,
            ["main_b_10a", "main_b_10b"],
            1,
        ),
    ] {
        let before = e.events().to_vec();
        let events = next_offer_of(&mut e, level, before);
        let offer = expertise_offer(&e);
        assert_eq!((offer.weapon, offer.kind), (WeaponRole::Main, kind));
        assert_eq!(candidates(&e), choices, "the chosen style's, fixed");
        assert_eq!(offer.options, takes(2), "no reroll");
        assert!(events.contains(&DomainEvent::ExpertiseOfferOpened {
            level,
            weapon: WeaponRole::Main,
            kind,
            candidates: ids(&choices),
        }));
        let hp = e.snapshot().player.hp;
        e.choose(pick).unwrap();
        assert_eq!(
            e.events().first(),
            Some(&DomainEvent::ExpertiseOptionTaken {
                weapon: WeaponRole::Main,
                option: choices[pick as usize].into(),
                taken: 1,
            })
        );
        assert_eq!(e.snapshot().player.hp, hp + 1, "the choice applies");
    }
    assert!(!e.offer_open());
    assert_eq!(
        gauge(&e).main_path,
        ids(&["main_b_6b", "main_b_9a", "main_b_10b"])
    );
}

#[test]
fn the_styles_and_the_path_taken_are_in_the_state_hash() {
    let make = || {
        with(
            &stage(Some((1, to_level(6))), vec![]),
            &styled(styles("main"), styles("sub")),
        )
    };
    let (mut a, mut b) = (make(), make());
    to_offer_of(&mut a, 3);
    to_offer_of(&mut b, 3);
    assert_eq!(a.state_hash(), b.state_hash());
    // Each style adds 1 percent more damage: only which one was chosen tells them apart.
    a.choose(0).unwrap();
    b.choose(1).unwrap();
    assert_ne!(a.state_hash(), b.state_hash(), "the style is hashed");
    let (mut c, mut d) = (make(), make());
    to_offer_of(&mut c, 6);
    to_offer_of(&mut d, 6);
    assert_eq!(c.state_hash(), d.state_hash());
    c.choose(0).unwrap();
    d.choose(1).unwrap();
    assert_ne!(
        c.state_hash(),
        d.state_hash(),
        "the upgrade taken is hashed"
    );
}

#[test]
fn an_inline_main_weapon_opens_no_style_upgrade_or_capstone_and_the_sub_still_does() {
    // An inline weapon has no expertise content: none of the Main's levels opens, its style
    // included, so 6, 9 and 10, which follow a Main style, fail closed and open nothing.
    let stage = armed(
        stage(Some((1, to_level(13))), vec![]),
        json!({ "inline": { "combo": { "main": { "attack": { "preset": { "id": "slash" } } } },
                            "window_ticks": 20, "assist_range": 40_960 } }),
        json!({ "preset": { "id": "maul" } }),
    );
    let mut e = with(&stage, &content(damages("m", 3), damages("s", 3)));
    let first = until_offer(&mut e);
    assert_eq!(
        take_all(&mut e, &first),
        ["boss", "sub 2", "sub 5", "sub 8", "sub 12"]
    );
    let g = gauge(&e);
    assert_eq!(g.level, 13);
    assert_eq!((g.main_style, g.sub_style), (None, Some("crush".into())));
    assert!(g.main_path.is_empty());
}

#[test]
fn with_both_common_pools_spent_the_style_path_still_opens_and_the_levels_after_pass_over() {
    let mut e = with(
        &stage(Some((1, to_level(13))), vec![]),
        &content(
            vec![option("quick", "attack_speed_pct", 10, Some(1))],
            vec![option("squick", "attack_speed_pct", 10, Some(1))],
        ),
    );
    let first = until_offer(&mut e);
    assert_eq!(
        take_all(&mut e, &first),
        [
            "boss", "main 1", "sub 2", "main 3", "sub 5", "main 6", "main 9", "main 10"
        ]
    );
    let g = gauge(&e);
    assert_eq!(g.level, 13);
    assert_eq!(
        g.main_path,
        ids(&["edge_6", "edge_9", "edge_10"]),
        "the bundled `blade`'s first style"
    );
}
