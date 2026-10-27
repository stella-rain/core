//! The expertise offers (ADR-106 §4 to §6; ADR-079 §2, §3): each level reached opens one offer
//! at the end of the tick, for the one weapon, in level order. The mode's level table places the
//! style path (normal: style 3, upgrades 6 and 9, capstone 10); every other level is a common
//! offer drawn from the weapon's common pool. The style level shows the weapon's three styles;
//! the upgrade levels and the capstone level the chosen style's fixed choices: no draw and no
//! reroll. A common offer has a reroll of each candidate; no expertise offer has a skip or a
//! banish. No option has a stack limit: one taken any number of times is still offered. On one
//! tick the boss offer opens first, then the expertise offers in level order, then the stars'.

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
};
use stella_rain_core::stage::{LevelTable, Stage};
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// 32 pixels above it: inside the `giant_sword`'s slash and the pickup circle.
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

/// A common option `id` of `effect` and `amount`.
fn option(id: &str, effect: &str, amount: u32) -> Value {
    json!({ "id": id, "effect": effect, "amount": amount,
            "title": "Option", "description": "" })
}

/// The damage options `prefix0`, `prefix1`, ...: `n` of them, 10 percent each.
fn damages(prefix: &str, n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| option(&format!("{prefix}{i}"), "attack_pct", 10))
        .collect()
}

/// The bundled content, its expertise with `TABLE` (growth 150 percent, at most 100_000), and
/// the `giant_sword`'s common pool replaced by `main`.
fn content(main: Vec<Value>) -> RunContent {
    content_with(TABLE, 150, 100_000, main)
}

fn content_with(
    thresholds: [u32; 10],
    growth_pct: u16,
    max_points: u32,
    main: Vec<Value>,
) -> RunContent {
    parsed(file_with(thresholds, growth_pct, max_points, main))
}

/// The bundled expertise file with `thresholds`, `growth_pct`, `max_points`, and the `giant_sword`'s
/// common pool replaced by `main`.
fn file_with(thresholds: [u32; 10], growth_pct: u16, max_points: u32, main: Vec<Value>) -> Value {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!(thresholds);
    file["growth_pct"] = json!(growth_pct);
    file["max_points"] = json!(max_points);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"].as_str() == Some("giant_sword") {
            w["common"] = json!(main.clone());
        }
    }
    file
}

fn parsed(file: Value) -> RunContent {
    let expertise = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test expertise validates");
    RunContent::bundled().clone().with_expertise(expertise)
}

/// An enemy of `hp` that never attacks, holding at `(x, y)`, giving `kill` points for its kill.
fn target(hp: u32, x: i32, y: i32, kill: u32) -> Value {
    let mut w = wave(1, 1, 0, enemy(hp, hold(x, y), json!([])), x, y);
    w["expertise"] = json!({ "hit": 0, "kill": kill });
    w
}

/// The `giant_sword` as the one weapon, levels of 1000 experience, stars that every kill
/// drops (taken at once within 48 pixels), one reroll and one banish; a boss of `boss_hp` 32
/// pixels ahead giving `boss_kill` points, or none; and `waves`.
fn stage(boss: Option<(u32, u32)>, waves: Vec<Value>) -> Stage {
    let boss = boss.map(|(hp, kill)| {
        let mut b = boss_at(hp, NEAR_Y);
        b["expertise"] = json!({ "hit": 0, "kill": kill });
        b
    });
    let mut stage = stage_full(18_000, boss.or(Some(far_boss())), waves, vec![]);
    stage = armed(stage, json!({ "preset": { "id": "giant_sword" } }));
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

/// As `stage`, with the mode's level table `table`.
fn stage_with_table(boss: Option<(u32, u32)>, table: LevelTable) -> Stage {
    let mut s = stage(boss, vec![]);
    s.level_table = Some(table);
    s
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

/// What each offer opening in `events` is: `boss`, `star`, or the expertise level.
fn openings(events: &[DomainEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::OfferOpened { kind, .. } => Some(match kind {
                OfferKind::Boss => "boss".to_string(),
                OfferKind::Star => "star".to_string(),
            }),
            DomainEvent::ExpertiseOfferOpened { level, .. } => Some(level.to_string()),
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

#[test]
fn a_boss_kill_crossing_several_thresholds_opens_one_offer_per_level_in_order() {
    let mut e = with(
        &stage(Some((1, to_level(13))), vec![]),
        &content(damages("m", 3)),
    );
    let first = until_offer(&mut e);
    let seen = take_all(&mut e, &first);
    // The style path (3, 6, 9, 10) opens in level order with the common offers.
    assert_eq!(
        seen,
        [
            "boss", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13"
        ]
    );
    let g = gauge(&e);
    assert_eq!((g.level, g.points), (13, 0));
    let sum = |t: &[u8]| t.iter().map(|n| u32::from(*n)).sum::<u32>();
    assert_eq!(sum(&g.taken), 9, "nine common offers, one pick each");
}

#[test]
fn an_expertise_offer_opens_at_the_end_of_the_tick_and_stops_the_world() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 3)),
    );
    let events = until_offer(&mut e);
    assert_eq!(
        openings(&events),
        ["1"],
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
        &content(damages("m", 3)),
    );
    let first = until_offer(&mut e);
    assert_eq!(take_all(&mut e, &first), ["1", "star"]);
}

#[test]
fn a_common_offer_holds_three_candidates_and_a_reroll_of_each_and_no_skip_or_banish() {
    use OfferOption::{Reroll, Take};
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 4)),
    );
    until_offer(&mut e);
    let offer = expertise_offer(&e);
    assert_eq!((offer.level, offer.kind), (1, ExpertiseOfferKind::Common));
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
        &content(damages("m", 4)),
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
        &content(damages("m", 3)),
    );
    until_offer(&mut e);
    assert_eq!(expertise_offer(&e).options.len(), 3);
}

#[test]
fn with_fewer_options_left_the_offer_holds_those() {
    let mut e = with(
        &stage(None, vec![target(1, START_X, NEAR_Y, 10)]),
        &content(damages("m", 2)),
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
        &content(vec![option("tough", "max_hp", 5)]),
    );
    until_offer(&mut e);
    let hp = e.snapshot().player.hp;
    e.choose(0).unwrap();
    assert_eq!(
        e.events().first(),
        Some(&DomainEvent::ExpertiseOptionTaken {
            option: "tough".into(),
            taken: 1,
        })
    );
    assert_eq!(e.snapshot().player.hp, hp + 5, "five more hit points");
    assert_eq!(gauge(&e).taken, [1]);
}

#[test]
fn an_attack_speed_option_is_offered_however_often_it_was_taken() {
    // One option in the pool: every level but 3, 6, 9 and 10 offers it.
    let main = vec![option("quick", "attack_speed_pct", 10)];
    let mut e = with(&stage(Some((1, to_level(16))), vec![]), &content(main));
    until_offer(&mut e);
    e.choose(0).unwrap(); // The boss offer.
    let mut quick_offers = 0;
    while e.offer_open() {
        let offer = expertise_offer(&e);
        if offer.candidates.iter().any(|c| c.option.0 == "quick") {
            quick_offers += 1;
        }
        e.choose(0).unwrap();
    }
    assert_eq!(quick_offers, 12, "offered at the 12 common levels of 16");
    assert_eq!(gauge(&e).taken, [u8::try_from(quick_offers).unwrap()]);
}

#[test]
fn a_boss_kill_no_longer_upgrades_a_weapon_its_points_are_its_weapon_progress() {
    let mut e = with(
        &stage(Some((1, to_level(1))), vec![]),
        &content(damages("m", 3)),
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
    assert_eq!(g.taken, [0, 0, 0]);
    assert_eq!(take_all(&mut e, &events), ["boss", "1"]);
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
        &content(damages("m", 3)),
    );
    let first = until_offer(&mut e);
    assert!(first.contains(&DomainEvent::StageCleared));
    assert!(
        first
            .iter()
            .any(|ev| matches!(ev, DomainEvent::StarTaken { .. }))
    );
    assert_eq!(take_all(&mut e, &first), ["boss", "1", "2", "star"]);
    assert_eq!(gauge(&e).level, 2);
}

#[test]
fn damage_stays_bounded_with_many_damage_options_stacked() {
    // Every level after the table needs 10 points: a kill's 100_000 points cross about 10_000
    // levels, each a pick of 100 percent more damage, which the arithmetic of ADR-056
    // holds without overflowing.
    let big = |p: &str| vec![option(p, "attack_pct", 100)];
    let content = content_with([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 101, 10, big("m"));
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
        g.taken,
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
            &content(damages("m", 4)),
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
    use crate::run_play::play_run;
    use common::run::{TRIAL_LENGTH, trial_setup};
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
    assert!(e.snapshot().expertise.unwrap().style.is_some());
    let replay = record_run(&trial_setup(), &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(hash_to_hex(verified.final_hash), replay.final_hash);
}

/// Style `id`, which adds `amount` of `effect` when chosen; its upgrades at 6 and 9 and its
/// capstones at 10 are two choices each, `<id>_6a` and `<id>_6b` and so on, a hit point each.
fn style(id: &str, effect: &str, amount: u32) -> Value {
    let two = |level: u32| {
        vec![
            option(&format!("{id}_{level}a"), "max_hp", 1),
            option(&format!("{id}_{level}b"), "max_hp", 1),
        ]
    };
    json!({ "id": id, "effect": effect, "amount": amount, "title": "Style", "description": "",
            "upgrade_1": two(6), "upgrade_2": two(9), "capstone": two(10) })
}

/// Styles `<prefix>_a`, `<prefix>_b` and `<prefix>_c`, each 1 percent more damage.
fn styles(prefix: &str) -> [Value; 3] {
    ["a", "b", "c"].map(|s| style(&format!("{prefix}_{s}"), "attack_pct", 1))
}

/// `content` with three damage options in the pool, and every weapon's styles `main`.
fn styled(main: [Value; 3]) -> RunContent {
    let mut file = file_with(TABLE, 150, 100_000, damages("m", 3));
    for w in file["weapons"].as_array_mut().unwrap() {
        w["styles"] = json!(main);
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
fn the_style_level_offers_the_three_styles_fixed_with_no_reroll_skip_or_banish() {
    let mut e = with(
        &stage(Some((1, to_level(3))), vec![]),
        &styled(styles("main")),
    );
    let events = to_offer_of(&mut e, 3);
    let offer = expertise_offer(&e);
    assert_eq!(offer.kind, ExpertiseOfferKind::Style);
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
        events.contains(&DomainEvent::ExpertiseOfferOpened {
            level: 3,
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
fn level_5_is_a_common_offer_there_is_no_second_style() {
    let mut e = with(
        &stage(Some((1, to_level(5))), vec![]),
        &styled(styles("main")),
    );
    to_offer_of(&mut e, 5);
    let offer = expertise_offer(&e);
    assert_eq!(offer.kind, ExpertiseOfferKind::Common);
    assert_eq!(offer.candidates.len(), 3);
    assert!(
        candidates(&e).iter().all(|c| c.starts_with('m')),
        "drawn from the pool: {:?}",
        candidates(&e)
    );
}

#[test]
fn choosing_a_style_applies_its_effect_and_holds_it_for_the_run() {
    let main = [
        style("main_a", "max_hp", 5),
        style("main_b", "max_hp", 7),
        style("main_c", "max_hp", 9),
    ];
    let mut e = with(&stage(Some((1, to_level(5))), vec![]), &styled(main));
    to_offer_of(&mut e, 3);
    let hp = e.snapshot().player.hp;
    e.choose(1).unwrap();
    assert_eq!(
        e.events().first(),
        Some(&DomainEvent::ExpertiseOptionTaken {
            option: "main_b".into(),
            taken: 1,
        })
    );
    assert_eq!(e.snapshot().player.hp, hp + 7, "seven more hit points");
    let g = gauge(&e);
    assert_eq!(g.style, Some("main_b".into()));
    assert!(g.path.is_empty());
}

/// The path of the chosen style at the levels of `table`: the style at `table.style`, then the
/// style's two upgrades and its capstone, each a fixed choice, never a draw.
fn walk_the_path(table: LevelTable) {
    let mut e = with(
        &stage_with_table(Some((1, to_level(10))), table),
        &styled(styles("main")),
    );
    to_offer_of(&mut e, table.style);
    assert_eq!(expertise_offer(&e).kind, ExpertiseOfferKind::Style);
    e.choose(1).unwrap();
    for (level, kind, choices, pick) in [
        (
            table.upgrades[0],
            ExpertiseOfferKind::Upgrade,
            ["main_b_6a", "main_b_6b"],
            1,
        ),
        (
            table.upgrades[1],
            ExpertiseOfferKind::Upgrade,
            ["main_b_9a", "main_b_9b"],
            0,
        ),
        (
            table.capstone,
            ExpertiseOfferKind::Capstone,
            ["main_b_10a", "main_b_10b"],
            1,
        ),
    ] {
        let before = e.events().to_vec();
        let events = next_offer_of(&mut e, level, before);
        let offer = expertise_offer(&e);
        assert_eq!(offer.kind, kind, "level {level}");
        assert_eq!(candidates(&e), choices, "the chosen style's, fixed");
        assert_eq!(offer.options, takes(2), "no reroll");
        assert!(events.contains(&DomainEvent::ExpertiseOfferOpened {
            level,
            kind,
            candidates: ids(&choices),
        }));
        let hp = e.snapshot().player.hp;
        e.choose(pick).unwrap();
        assert_eq!(
            e.events().first(),
            Some(&DomainEvent::ExpertiseOptionTaken {
                option: choices[pick as usize].into(),
                taken: 1,
            })
        );
        assert_eq!(e.snapshot().player.hp, hp + 1, "the choice applies");
    }
    // Every other level before the capstone is a common offer: the four path levels are not.
    assert_eq!(
        gauge(&e).taken.iter().map(|n| u32::from(*n)).sum::<u32>(),
        table.capstone - 4,
    );
    assert_eq!(
        gauge(&e).path,
        ids(&["main_b_6b", "main_b_9a", "main_b_10b"])
    );
}

#[test]
fn the_normal_table_places_the_style_at_3_upgrades_at_6_and_9_and_the_capstone_at_10() {
    walk_the_path(LevelTable::NORMAL);
}

#[test]
fn the_short_table_places_the_style_at_2_upgrades_at_4_and_6_and_the_capstone_at_8() {
    walk_the_path(LevelTable::SHORT);
}

#[test]
fn the_styles_and_the_path_taken_are_in_the_state_hash() {
    let make = || {
        with(
            &stage(Some((1, to_level(6))), vec![]),
            &styled(styles("main")),
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
fn an_inline_weapon_has_no_expertise_content() {
    // An inline weapon has no expertise content: no offer opens, its styles and upgrades
    // included, so the upgrade and capstone levels, which follow a style, fail closed.
    let stage = armed(
        stage(Some((1, to_level(13))), vec![]),
        json!({ "inline": { "combo": { "main": { "attack": { "preset": { "id": "slash" } } } },
                            "window_ticks": 20, "assist_range": 40_960 } }),
    );
    let mut e = with(&stage, &content(damages("m", 3)));
    let first = until_offer(&mut e);
    assert_eq!(take_all(&mut e, &first), ["boss"]);
    let g = gauge(&e);
    assert_eq!(g.level, 13);
    assert_eq!(g.style, None);
    assert!(g.path.is_empty());
}
