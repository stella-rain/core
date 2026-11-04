//! The offer mode of a run: `stop` and `timed` (ADR-SIM-7 point 4, ADR-REPL-5).
use crate::common;
use stella_rain_core::engine::Engine;
use stella_rain_core::engine::Outcome;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, RunReplayError, verify_run};
use stella_rain_core::run_setup::{OfferMode, RunSetup};

/// The event a pick of a star offer or a stat offer leaves.
fn is_pick(ev: &DomainEvent) -> bool {
    matches!(
        ev,
        DomainEvent::RewardTaken { .. } | DomainEvent::StatRaised { .. }
    )
}

/// The open offers as the snapshot shows them.
fn offer_views(e: &Engine) -> String {
    let p = e.snapshot().progress.unwrap();
    serde_json::to_string(&(p.stat_offer, p.offer)).unwrap()
}

fn d() -> Input {
    Input::default()
}

fn timed() -> RunSetup {
    RunSetup {
        pick_stats: true,
        offer_mode: OfferMode::Timed,
        ..RunSetup::default()
    }
}

/// One step with the scripted bot and `pick` for slot 0.
fn bot_step(e: &mut Engine, pick: Option<u32>) {
    let mut i = common::run::run_input(e, e.tick());
    if let Some(p) = pick {
        i.pick = Some(p);
    }
    e.step(&[i, d(), d()]);
}

/// Plays `e` with the bot until the first offer is open; returns the tick it opened on.
fn until_offer(e: &mut Engine) -> u32 {
    while !e.offer_open() {
        assert!(e.tick() < 3000, "no offer before tick 3000");
        bot_step(e, None);
    }
    e.tick()
}

// --- A. settings --------------------------------------------------------------------------------

#[test]
fn the_default_setup_is_stop_and_names_no_mode() {
    let s = RunSetup::default();
    assert_eq!(s.offer_mode, OfferMode::Stop);
    assert!(!serde_json::to_string(&s).unwrap().contains("offer_mode"));
}

#[test]
fn the_mode_reads_strictly_and_stop_is_skipped() {
    let base = serde_json::to_value(RunSetup::default()).unwrap();
    let with = |v: serde_json::Value| {
        let mut o = base.clone();
        o["offer_mode"] = v;
        serde_json::from_value::<RunSetup>(o)
    };
    let t = with("timed".into()).unwrap();
    assert_eq!(t.offer_mode, OfferMode::Timed);
    assert!(
        serde_json::to_string(&t)
            .unwrap()
            .contains("\"offer_mode\":\"timed\"")
    );
    let s = with("stop".into()).unwrap();
    assert_eq!(s.offer_mode, OfferMode::Stop);
    assert!(!serde_json::to_string(&s).unwrap().contains("offer_mode"));
    assert!(with("fast".into()).is_err());
    assert!(with(serde_json::Value::Null).is_err());
}

#[test]
fn a_timed_setup_needs_sim_version_2() {
    use stella_rain_core::run_setup::validate_in;
    use stella_rain_core::validate::Error;
    let one = RunSetup {
        sim_version: 1,
        ..timed()
    };
    match validate_in(&one, stella_rain_core::run_content::RunContent::bundled()) {
        Err(Error::Invalid(issues)) => assert!(issues.iter().any(|i| i.path == "$.offer_mode")),
        other => panic!("expected $.offer_mode, got {other:?}"),
    }
    let two = RunSetup {
        sim_version: 2,
        ..timed()
    };
    assert_eq!(
        validate_in(&two, stella_rain_core::run_content::RunContent::bundled()),
        Ok(())
    );
}

#[test]
fn the_bundled_timer_is_300_and_its_bounds_are_checked() {
    use stella_rain_core::rules::{MAX_OFFER_TIMER_TICKS, Rules};
    assert_eq!(Rules::bundled().offer_timer_ticks(), 300);
    assert_eq!(MAX_OFFER_TIMER_TICKS, 3600);
    let bundled: serde_json::Value =
        serde_json::from_slice(stella_rain_core::pack::ContentFiles::bundled_file("rules"))
            .unwrap();
    let versions = stella_rain_core::version::Versions::CURRENT;
    let with = |n: u32| {
        let mut v = bundled.clone();
        v["offer_timer_ticks"] = serde_json::json!(n);
        Rules::parse(v.to_string().as_bytes(), versions)
    };
    assert!(with(1).is_ok());
    assert!(with(3600).is_ok());
    for bad in [0, 3601] {
        let msg = format!("{:?}", with(bad).unwrap_err());
        assert!(msg.contains("offer_timer_ticks"), "{msg}");
    }
}

// --- B. stop is untouched -------------------------------------------------------------------------

/// The state hash of the stop engine after 600 bot steps (a pick is a step), recorded on the base
/// build before the mode existed and re-pinned for the per-player layout and the slot generations.
const STOP_HASH_600: u64 = 13_949_107_820_038_227_188;

#[test]
fn stop_is_byte_identical_to_the_base() {
    let mut e = Engine::start(&RunSetup::default()).unwrap();
    assert_eq!(e.offer_mode(), OfferMode::Stop);
    for _ in 0..600 {
        if e.offer_open() {
            common::pick(&mut e, 0).unwrap();
        } else {
            bot_step(&mut e, None);
        }
    }
    assert_eq!((e.state_hash(), e.tick()), (STOP_HASH_600, 595));
}

#[test]
fn stop_with_an_offer_open_stops_the_world() {
    let mut e = Engine::start(&RunSetup {
        pick_stats: true,
        ..RunSetup::default()
    })
    .unwrap();
    let t = until_offer(&mut e);
    let h = e.state_hash();
    e.step(&[d(), d(), d()]);
    assert_eq!((e.tick(), e.state_hash()), (t, h));
    e.step(&[Input::picking(0), d(), d()]);
    assert_eq!(e.tick(), t);
    assert!(!e.offer_open() || e.state_hash() != h);
    let mut r = RunRecorder::start(&RunSetup {
        pick_stats: true,
        ..RunSetup::default()
    })
    .unwrap();
    while !r.engine().offer_open() {
        let i = common::run::run_input(r.engine(), r.engine().tick());
        r.step(&[i, d(), d()]).unwrap();
    }
    let tick = r.engine().tick();
    assert!(matches!(
        r.step(&[d(), d(), d()]),
        Err(RunReplayError::OfferOpen { tick: t }) if t == tick
    ));
}

// --- C. timed -------------------------------------------------------------------------------------

#[test]
fn a_timed_world_keeps_going_with_an_offer_open() {
    let mut e = Engine::start(&timed()).unwrap();
    assert_eq!(e.offer_mode(), OfferMode::Timed);
    let t = until_offer(&mut e);
    let offer = offer_views(&e);
    let mut seen = vec![e.state_hash()];
    for k in 1..=10 {
        e.step(&[d(), d(), d()]);
        assert_eq!(e.tick(), t + k);
        assert!(e.offer_open());
        assert_eq!(offer_views(&e), offer);
        assert!(!seen.contains(&e.state_hash()));
        seen.push(e.state_hash());
    }
}

#[test]
fn the_timer_picks_option_0_on_the_300th_step() {
    let mut e = Engine::start(&timed()).unwrap();
    let t = until_offer(&mut e);
    for k in 1..300 {
        e.step(&[d(), d(), d()]);
        assert_eq!(e.tick(), t + k);
        assert!(e.offer_open(), "still open at +{k}");
    }
    // The same state, answered by a manual pick of option 0 on its own step.
    let mut manual = Engine::start(&timed()).unwrap();
    until_offer(&mut manual);
    manual.step(&[Input::picking(0), d(), d()]);
    let want: Vec<_> = manual
        .events()
        .iter()
        .filter(|ev| is_pick(ev))
        .cloned()
        .collect();
    assert_eq!(want.len(), 1);
    e.step(&[d(), d(), d()]);
    assert_eq!(e.tick(), t + 300);
    let got: Vec<_> = e
        .events()
        .iter()
        .filter(|ev| is_pick(ev))
        .cloned()
        .collect();
    assert_eq!(got, want);
}

#[test]
fn a_pick_beats_the_timer_of_the_same_step() {
    let mut e = Engine::start(&timed()).unwrap();
    let t = until_offer(&mut e);
    for _ in 1..300 {
        e.step(&[d(), d(), d()]);
    }
    e.step(&[Input::picking(1), d(), d()]);
    assert_eq!(e.tick(), t + 300);
    let raised = e.events().iter().filter(|ev| is_pick(ev)).count();
    assert_eq!(raised, 1, "one pick, not two");
    // A pick for nothing, or out of range, changes nothing.
    let mut a = Engine::start(&timed()).unwrap();
    until_offer(&mut a);
    let mut b = Engine::start(&timed()).unwrap();
    until_offer(&mut b);
    a.step(&[Input::picking(99), d(), d()]);
    b.step(&[d(), d(), d()]);
    assert_eq!(a.state_hash(), b.state_hash());
}

// --- D. recorder and verifier ---------------------------------------------------------------------

#[test]
fn a_timed_recorder_steps_with_an_offer_open_and_verifies() {
    let mut r = RunRecorder::start(&timed()).unwrap();
    while !r.engine().offer_open() {
        let i = common::run::run_input(r.engine(), r.engine().tick());
        r.step(&[i, d(), d()]).unwrap();
    }
    let t = r.engine().tick();
    r.step(&[d(), d(), d()]).unwrap();
    assert_eq!(r.engine().tick(), t + 1);
    r.step(&[Input::picking(0), d(), d()]).unwrap();
    assert_eq!(r.engine().tick(), t + 2);
    let v = verify_run(&r.replay().to_bytes()).unwrap();
    assert_eq!(v.commands, 1);
    assert_eq!(v.final_hash, r.engine().state_hash());
}

#[test]
fn offers_left_to_the_timer_record_no_pick_and_verify() {
    let mut r = RunRecorder::start(&timed()).unwrap();
    let (mut opened, mut raised) = (0, 0);
    for _ in 0..900 {
        let i = common::run::run_input(r.engine(), r.engine().tick());
        r.step(&[i, d(), d()]).unwrap();
        opened += usize::from(r.engine().offer_open());
        raised += r.engine().events().iter().filter(|ev| is_pick(ev)).count();
    }
    assert!(opened > 0 && raised > 0);
    let v = verify_run(&r.replay().to_bytes()).unwrap();
    assert_eq!(v.commands, 0);
    assert_eq!(v.final_hash, r.engine().state_hash());
    assert_eq!(r.engine().outcome(), Outcome::Running);
}

#[test]
fn a_timed_run_saves_and_resumes_with_an_offer_open() {
    use stella_rain_core::pack::Pack;
    let mut r = RunRecorder::start(&timed()).unwrap();
    // Picks at some offers, the timer at others.
    let mut opened = 0;
    for k in 0..700 {
        let mut i = common::run::run_input(r.engine(), r.engine().tick());
        if r.engine().offer_open() && k % 2 == 0 && opened % 2 == 0 {
            i.pick = Some(0);
        }
        opened += usize::from(r.engine().offer_open());
        r.step(&[i, d(), d()]).unwrap();
    }
    while !r.engine().offer_open() {
        let i = common::run::run_input(r.engine(), r.engine().tick());
        r.step(&[i, d(), d()]).unwrap();
    }
    r.step(&[d(), d(), d()]).unwrap();
    assert!(r.engine().offer_open());
    let (bytes, prefix) = (r.save(), r.replay());
    let mut back = RunRecorder::resume_from_savepoint(&bytes, &prefix, Pack::bundled()).unwrap();
    assert_eq!(back.engine().state_hash(), r.engine().state_hash());
    assert_eq!(back.save(), bytes);
    for _ in 0..150 {
        let i = common::run::run_input(r.engine(), r.engine().tick());
        r.step(&[i, d(), d()]).unwrap();
        back.step(&[i, d(), d()]).unwrap();
    }
    assert_eq!(back.engine().state_hash(), r.engine().state_hash());
    assert_eq!(back.replay().to_bytes(), r.replay().to_bytes());
    assert!(verify_run(&r.replay().to_bytes()).is_ok());
}
