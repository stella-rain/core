//! Runes (ADR-099 points 1, 2, 4, 7, 8): a closed list built into `core`, held by the Main
//! weapon each at most once, and the random rune, a star reward that shows as a hidden
//! candidate and draws its rune from the run's generator only when it is picked, among the
//! runes not held. Once every rune is held it leaves the pool. No rune does anything yet.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::ContentId;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::{RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::rune::{MAX_RUNES, Rune, RuneKind};
use stella_rain_core::snapshot::{OfferOption, RuneView};
use stella_rain_core::stage::Stage;

mod common;

use common::run::{run_input, trial_setup};
use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the first step's hit box.
const NEAR_Y: i32 = START_Y - 8_192;

/// A Main weapon of one step: a circle of 32 pixels 4 pixels ahead, one active tick of 10
/// damage and 4 ticks of recovery.
fn weapon() -> Value {
    json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

/// `count` enemies of 10 hp (one step each), one every 6 ticks from tick 1, at `NEAR_Y`; every
/// kill drops a common star, taken at once; `rerolls` and `banishes` for the run.
fn targets(count: u16, rerolls: u8, banishes: u8) -> Stage {
    let waves = vec![wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
        quiet_sub(),
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": rerolls, "banishes": banishes,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// A common random rune.
fn random_rune() -> Value {
    json!({
        "id": "random_rune", "rarity": "common", "effect": "random_rune",
        "title": "Rune", "description": ""
    })
}

/// A common reward `id` that changes nothing in the test stages: an evade charge, while the
/// stage's evade has no more charges to give.
fn inert(id: &str) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "evade_charge", "amount": 1, "limit": 20,
        "title": "Nothing", "description": ""
    })
}

fn file(rewards: Vec<Value>) -> String {
    json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards }).to_string()
}

fn rewards(list: Vec<Value>) -> Rewards {
    Rewards::parse(file(list).as_bytes()).expect("the test rewards validate")
}

fn content(list: Vec<Value>) -> RunContent {
    RunContent::new(RunStats::bundled().clone(), rewards(list))
}

fn with(stage: &Stage, list: Vec<Value>) -> Engine {
    Engine::new_with(stage, &content(list)).expect("the test stage is valid")
}

/// Fires until the next offer opens.
fn next_offer(e: &mut Engine) {
    fire_until(e, idle(), 600, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 600 ticks");
}

fn offered(e: &Engine) -> Vec<String> {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let mut ids: Vec<String> = offer
        .candidates
        .iter()
        .map(|c| c.reward.0.clone())
        .collect();
    ids.sort();
    ids
}

/// The place of `reward` among the open offer's candidates.
fn slot(e: &Engine, reward: &str) -> u8 {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let at = offer
        .candidates
        .iter()
        .position(|c| c.reward.0 == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    u8::try_from(at).unwrap()
}

/// Chooses `option` of the open offer.
fn pick(e: &mut Engine, option: OfferOption) {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let index = offer
        .options
        .iter()
        .position(|o| *o == option)
        .unwrap_or_else(|| panic!("{option:?} is not an option"));
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

fn take(e: &mut Engine, reward: &str) {
    let candidate = slot(e, reward);
    pick(e, OfferOption::Take { candidate });
}

fn explosion() -> RuneView {
    RuneView {
        rune: Rune::Explosion,
        cooldown_left: 0,
    }
}

#[test]
fn the_runes_are_a_closed_list_of_two_kinds() {
    assert_eq!(Rune::ALL, [Rune::Explosion]);
    assert_eq!(MAX_RUNES, Rune::ALL.len());
    assert_eq!(Rune::Explosion.kind(), RuneKind::OnHit);
    assert_eq!(serde_json::to_value(Rune::Explosion).unwrap(), "explosion");
}

#[test]
fn a_random_rune_has_a_rarity_and_no_amount_or_limit() {
    let parsed = rewards(vec![random_rune()]);
    let r = &parsed.rewards()[0];
    assert_eq!(r.effect, RewardEffect::RandomRune);
    assert_eq!((r.amount, r.limit), (None, None));
    let refused = |field: &str, value: Value| {
        let mut reward = random_rune();
        reward[field] = value;
        let err = Rewards::parse(file(vec![reward]).as_bytes()).unwrap_err();
        format!("{err:?}").contains(&format!("$.rewards[0].{field}"))
    };
    assert!(refused("amount", json!(1)), "an amount");
    assert!(refused("limit", json!(1)), "a limit");
}

#[test]
fn every_other_reward_still_needs_its_amount_and_limit() {
    for field in ["amount", "limit"] {
        let mut reward = inert("dash");
        reward.as_object_mut().unwrap().remove(field);
        let err = Rewards::parse(file(vec![reward]).as_bytes()).unwrap_err();
        assert!(
            format!("{err:?}").contains(&format!("$.rewards[0].{field}")),
            "{field} missing: {err:?}"
        );
    }
}

#[test]
fn the_hidden_candidate_names_no_rune() {
    let mut e = with(&targets(1, 0, 0), vec![random_rune(), inert("dash")]);
    next_offer(&mut e);
    let snapshot = e.snapshot();
    let offer = snapshot.progress.as_ref().unwrap().offer.as_ref().unwrap();
    for c in &offer.candidates {
        assert_eq!(c.hidden, c.reward.0 == "random_rune", "{}", c.reward.0);
    }
    assert!(snapshot.player.runes.is_empty(), "nothing drawn yet");
    let text = serde_json::to_string(&snapshot).unwrap();
    assert!(text.contains("\"hidden\":true"));
    assert!(!text.contains("explosion"), "no rune in the snapshot");
}

#[test]
fn the_pick_draws_a_rune_not_held_and_the_event_carries_it() {
    let mut e = with(&targets(1, 0, 0), vec![random_rune(), inert("dash")]);
    next_offer(&mut e);
    take(&mut e, "random_rune");
    assert_eq!(
        e.events(),
        [DomainEvent::RewardTaken {
            reward: ContentId("random_rune".into()),
            taken: 1,
            rune: Some(Rune::Explosion),
        }]
    );
    assert_eq!(e.snapshot().player.runes, [explosion()]);
}

#[test]
fn a_reward_without_a_rune_carries_none_in_its_event() {
    let mut e = with(&targets(1, 0, 0), vec![random_rune(), inert("dash")]);
    next_offer(&mut e);
    take(&mut e, "dash");
    assert!(matches!(
        e.events(),
        [DomainEvent::RewardTaken { rune: None, .. }]
    ));
    assert!(e.snapshot().player.runes.is_empty());
}

#[test]
fn with_every_rune_held_the_random_rune_leaves_the_pool() {
    // One rune in the list: after the first pick the Main holds them all.
    let mut e = with(&targets(3, 0, 0), vec![random_rune(), inert("dash")]);
    next_offer(&mut e);
    assert_eq!(offered(&e), ["dash", "random_rune"]);
    take(&mut e, "random_rune");
    next_offer(&mut e);
    assert_eq!(offered(&e), ["dash"]);
    take(&mut e, "dash");
    next_offer(&mut e);
    assert_eq!(offered(&e), ["dash"]);
    assert_eq!(e.snapshot().player.runes, [explosion()], "held once");
}

#[test]
fn a_random_rune_alone_with_every_rune_held_opens_no_offer() {
    let mut e = with(&targets(2, 0, 0), vec![random_rune()]);
    next_offer(&mut e);
    take(&mut e, "random_rune");
    fire_until(&mut e, idle(), 120, Engine::offer_open);
    assert!(!e.offer_open(), "nothing left to offer");
}

#[test]
fn a_random_rune_candidate_is_rerolled_like_any_reward() {
    let list = vec![random_rune(), inert("a"), inert("b"), inert("c")];
    let mut e = with(&targets(2, 1, 0), list);
    next_offer(&mut e);
    let candidate = slot(&e, "random_rune");
    pick(&mut e, OfferOption::Reroll { candidate });
    assert!(e.offer_open());
    assert_eq!(offered(&e), ["a", "b", "c"]);
    assert!(e.snapshot().player.runes.is_empty(), "no rune drawn");
    // It stays in the pool.
    take(&mut e, "a");
    next_offer(&mut e);
    assert!(offered(&e).contains(&"random_rune".to_string()));
}

#[test]
fn banishing_the_random_rune_removes_it_for_the_run() {
    let list = vec![random_rune(), inert("a"), inert("b"), inert("c")];
    let mut e = with(&targets(4, 0, 1), list);
    next_offer(&mut e);
    let candidate = slot(&e, "random_rune");
    pick(&mut e, OfferOption::Banish { candidate });
    assert!(e.events().contains(&DomainEvent::RewardBanished {
        reward: ContentId("random_rune".into())
    }));
    assert_eq!(offered(&e), ["a", "b", "c"]);
    for _ in 0..3 {
        take(&mut e, "a");
        next_offer(&mut e);
        assert_eq!(offered(&e), ["a", "b", "c"], "banished for the run");
    }
    assert!(e.snapshot().player.runes.is_empty());
}

#[test]
fn nothing_is_drawn_before_the_pick() {
    // The same pool, but the first reward a random rune in one run and an inert reward in the
    // other: an offer holding it, rerolled away and then skipped, leaves the same state.
    let rest = || vec![inert("a"), inert("b"), inert("c")];
    let mut list_a = vec![random_rune()];
    list_a.extend(rest());
    let mut list_b = vec![inert("z")];
    list_b.extend(rest());
    let stage = targets(2, 1, 0);
    let mut a = with(&stage, list_a);
    let mut b = with(&stage, list_b);
    next_offer(&mut a);
    next_offer(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    let candidate = slot(&a, "random_rune");
    assert_eq!(slot(&b, "z"), candidate);
    pick(&mut a, OfferOption::Reroll { candidate });
    pick(&mut b, OfferOption::Reroll { candidate });
    assert_eq!(a.state_hash(), b.state_hash(), "a reroll draws no rune");
    pick(&mut a, OfferOption::Skip);
    pick(&mut b, OfferOption::Skip);
    for _ in 0..30 {
        a.step(idle());
        b.step(idle());
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn the_state_hash_holds_the_runes() {
    // Taking the random rune and taking the inert reward in its place differ in the rune held
    // and in the draw.
    let rest = || vec![inert("a"), inert("b"), inert("c")];
    let mut list_a = vec![random_rune()];
    list_a.extend(rest());
    let mut list_b = vec![inert("z")];
    list_b.extend(rest());
    let stage = targets(1, 0, 0);
    let mut a = with(&stage, list_a);
    let mut b = with(&stage, list_b);
    next_offer(&mut a);
    next_offer(&mut b);
    take(&mut a, "random_rune");
    take(&mut b, "z");
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_with_a_random_rune_picked_records_and_verifies() {
    // The trial with a random rune and one stat reward: the first offer holds the rune.
    let content = content(vec![
        random_rune(),
        json!({
            "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5,
            "limit": 20, "title": "Might", "description": ""
        }),
    ]);
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..1_200 {
        while recorder.engine().offer_open() {
            let offer = recorder.engine().snapshot().progress.unwrap().offer.unwrap();
            let rune = offer.candidates.iter().position(|c| c.hidden);
            recorder
                .choose(u32::try_from(rune.unwrap_or(0)).unwrap())
                .unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
    }
    assert_eq!(
        recorder.engine().snapshot().player.runes,
        [explosion()],
        "the random rune was picked"
    );
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
