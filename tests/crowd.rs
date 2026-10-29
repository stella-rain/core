//! The crowd target, the cap derived from it and the spawn director (DIR1, DS103): the pack's
//! `rules.json` holds the target (200) and the cap's headroom (30%); a stage or the `time_limit`
//! mode may override the target; the cap is `target * (100 + headroom) / 100` (260), never more
//! than the engine ceiling of 500. In horde runs a wave spawn that falls due at the target is held.
//!
//! The bots are EC1's: `stand` (no move, Main on even ticks) and `circle` (move 100 round a
//! 600-tick circle, Main on even ticks, Evade in the first 2 ticks of every 45).

use crate::common;
use stella_rain_core::version::Versions;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::bench_fixtures::worst_minute_case::Run;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::trig;

// --- Stages -----------------------------------------------------------------------------------

/// `waves` waves of 100 enemies that stand still, each spawning one a tick from tick 1.
fn crowd_stage(waves: usize, crowd_target: Option<u64>) -> Stage {
    let stage = quiet(stage_full(
        18_000,
        Some(far_boss()),
        (0..waves)
            .map(|_| {
                wave(
                    1,
                    100,
                    1,
                    enemy(50, hold(46_080, 50_000), json!([])),
                    46_080,
                    50_000,
                )
            })
            .collect(),
        vec![],
    ));
    let mut v = serde_json::to_value(&stage).expect("a stage serializes");
    if let Some(target) = crowd_target {
        v["crowd_target"] = target.into();
    }
    serde_json::from_value(v).expect("the stage parses")
}

/// Alive enemies, and the spawns refused so far, `ticks` ticks into `stage` (standing still).
fn stand_in(stage: &Stage, ticks: u32) -> (usize, u32) {
    let mut e = engine(stage);
    let mut refused = 0;
    for _ in 0..ticks {
        e.step(idle());
        refused += e.load().refused_spawns;
    }
    (enemies(&e.snapshot()), refused)
}

// --- The `time_limit` mode --------------------------------------------------------------------

fn setup(difficulty: &str) -> RunSetup {
    RunSetup {
        mode: "time_limit".into(),
        seed: 1,
        difficulty: difficulty.into(),
        ..RunSetup::default()
    }
}

/// The bundled mode, changed by `edit`.
fn mode(edit: impl FnOnce(&mut Value)) -> TimeLimit {
    let mut file: Value =
        serde_json::from_slice(common::bundled_file("time_limit")).expect("the bundled mode");
    edit(&mut file);
    TimeLimit::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the mode parses")
}

fn horde(difficulty: &str, mode: Option<TimeLimit>) -> Engine {
    let mut content = design_content();
    if let Some(mode) = mode {
        content = content.with_time_limit(mode);
    }
    Engine::start_with(&setup(difficulty), &content).expect("the run starts")
}

/// The bundled mode's schedule made dense: every wave starts at tick 1 and spends 1.4 times its
/// count over 14,000 ticks, the elite cap is `elite_cap` and the first boss comes at tick 7,000.
fn dense(elite_cap: u64, edit: impl FnOnce(&mut Value)) -> TimeLimit {
    mode(|file| {
        for wave in file["waves"].as_array_mut().expect("waves") {
            let count = wave["count"].as_u64().expect("a count") * 14 / 10;
            wave["count"] = count.into();
            wave["at_tick"] = 1.into();
            wave["every_ticks"] = (14_000 / count).max(1).into();
        }
        file["boss_ticks"] = json!([7_000, file["length_ticks"]]);
        file["no_call_ticks"] = json!([]);
        file["elite_cap"] = elite_cap.into();
        edit(file);
    })
}

/// Takes the first option of an open offer.
fn answer(e: &mut Engine) {
    while e.offer_open() {
        e.choose(0).expect("the first option");
    }
}

fn stand(tick: u32) -> Input {
    Input {
        main: tick % 2 == 0,
        ..Input::default()
    }
}

fn circle(tick: u32) -> Input {
    let (sin, cos) =
        trig::sin_cos(i32::try_from(tick % 600).unwrap() * 360 * trig::PER_DEGREE / 600);
    let toward = |unit: i32| i8::try_from(100 * i64::from(unit) / i64::from(trig::SCALE)).unwrap();
    Input {
        move_x: toward(cos),
        move_y: toward(sin),
        main: tick % 2 == 0,
        evade: tick % 45 < 2,
        ..Input::default()
    }
}

/// The most enemies a bot's run held in any tick up to `ticks`, the spawns refused and the
/// wave spawns the director held.
fn peak(e: &mut Engine, ticks: u32, bot: fn(u32) -> Input) -> (u32, u32, u32) {
    let (mut most, mut refused, mut held) = (0, 0, 0);
    for _ in 0..ticks {
        answer(e);
        e.step(bot(e.tick()));
        most = most.max(e.load().enemies);
        refused += e.load().refused_spawns;
        held += e.load().held_spawns;
    }
    (most, refused, held)
}

// --- The cap ----------------------------------------------------------------------------------

#[test]
fn a_stage_stops_at_the_derived_cap() {
    // 300 spawns against a cap of 260 (200 * 130 / 100): 40 are refused.
    assert_eq!(stand_in(&crowd_stage(3, None), 101), (260, 40));
}

#[test]
fn a_stage_has_no_director_so_its_waves_keep_their_timing() {
    // Target 100, cap 130: every wave still spawns on its tick (the surplus is refused at the
    // cap and counted), and the director holds nothing.
    let mut e = engine(&crowd_stage(3, Some(100)));
    let (mut refused, mut held) = (0, 0);
    for _ in 0..101 {
        e.step(idle());
        refused += e.load().refused_spawns;
        held += e.load().held_spawns;
    }
    assert_eq!((enemies(&e.snapshot()), refused, held), (130, 170, 0));
}

#[test]
fn a_stage_override_moves_the_cap_with_the_target() {
    // 100 * 130 / 100 = 130 of 300 spawns.
    assert_eq!(stand_in(&crowd_stage(3, Some(100)), 101), (130, 170));
}

#[test]
fn a_stage_override_is_clamped_so_the_cap_stays_within_the_ceiling() {
    // 400 is clamped to 500 * 100 / 130 = 384 at headroom 30, whose cap is 499 of 600 spawns.
    assert_eq!(stand_in(&crowd_stage(6, Some(400)), 101), (499, 101));
}

#[test]
fn a_circle_run_never_holds_more_than_the_cap() {
    for difficulty in ["normal", "rainy"] {
        let mut e = horde(difficulty, None);
        let (most, _, held) = peak(&mut e, 20_000, circle);
        assert!(most <= 260, "{difficulty}: {most} enemies alive");
        assert!(held > 0, "{difficulty}: the director held nothing");
    }
}

#[test]
fn the_dense_minute_holds_the_cap_and_still_brings_its_elites() {
    let mut run = Run::fast_forwarded(0);
    let mut elite_ticks = Vec::new();
    for _ in 0..10_900 {
        run.play(1);
        let e = &run.engine;
        assert!(
            e.load().enemies <= 260,
            "tick {}: {}",
            e.tick(),
            e.load().enemies
        );
        if e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::EliteSpawned { .. }))
        {
            elite_ticks.push(e.tick());
        }
    }
    assert!(elite_ticks.contains(&3_600), "{elite_ticks:?}");
    assert!(elite_ticks.contains(&7_200), "{elite_ticks:?}");
}

// --- The director -----------------------------------------------------------------------------

/// Enemies alive in each tick from the first one that has `target`, up to tick 6,000, of a dense
/// run with one elite at most, no boss before then and no Main weapon pressed.
fn held_at(mode: TimeLimit, target: usize) -> Vec<usize> {
    let content = design_content().with_time_limit(mode);
    let mut e = Engine::start_with(&setup("normal"), &content).expect("the run starts");
    let mut alive = Vec::new();
    for _ in 0..6_000 {
        answer(&mut e);
        e.step(idle());
        let now = enemies(&e.snapshot());
        if !alive.is_empty() || now >= target {
            alive.push(now);
        }
    }
    alive
}

fn assert_held_at(alive: &[usize], target: usize) {
    assert!(!alive.is_empty(), "the crowd never reached {target}");
    assert!(
        alive.iter().all(|&n| n <= target + 1),
        "{:?}",
        alive.iter().max()
    );
    let near = alive.iter().filter(|&&n| n + 1 >= target).count();
    assert!(
        near * 10 >= alive.len() * 9,
        "{near} of {} near",
        alive.len()
    );
}

#[test]
fn horde_waves_stop_at_the_target() {
    assert_held_at(&held_at(dense(1, |_| {}), 200), 200);
}

#[test]
fn a_mode_override_moves_the_target() {
    let alive = held_at(dense(1, |file| file["crowd_target"] = 150.into()), 150);
    assert_held_at(&alive, 150);
}

// --- Under the target nothing moves ------------------------------------------------------------

fn fnv1a(hash: u64, word: u32) -> u64 {
    word.to_le_bytes().iter().fold(hash, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The first tick the standing bot's crowd (counted as `Load::enemies` is) has the 200 of the
/// target, taken on main after VS1 (DS109): the director first acts on it, so every tick before it is
/// what it always was.
const T200: u32 = 36_180;
/// FNV-1a 64 of each tick's `(tick, enemies, refused_spawns, relocated)` for the ticks before
/// `T200` of the standing bot's run, taken on main after VS1 (DS109).
const UNDER_TARGET_DIGEST: u64 = 0xdf41_69bd_d805_7606;

#[test]
fn under_the_target_a_run_is_what_it_was() {
    let mut e = horde("normal", None);
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let mut t200 = None;
    for _ in 0..90_000 {
        answer(&mut e);
        e.step(stand(e.tick()));
        let load = e.load();
        if load.enemies >= 200 {
            t200 = Some(e.tick());
            break;
        }
        assert_eq!(load.held_spawns, 0, "tick {}", e.tick());
        for word in [e.tick(), load.enemies, load.refused_spawns, load.relocated] {
            digest = fnv1a(digest, word);
        }
    }
    assert_eq!((t200, digest), (Some(T200), UNDER_TARGET_DIGEST));
}

// --- Engine::caps -----------------------------------------------------------------------------

#[test]
fn the_engine_reports_the_cap_of_its_run() {
    assert_eq!(engine(&crowd_stage(1, None)).caps().enemies, 260);
    assert_eq!(engine(&crowd_stage(1, Some(100))).caps().enemies, 130);
    assert_eq!(engine(&crowd_stage(1, Some(400))).caps().enemies, 499);
    assert_eq!(horde("normal", None).caps().enemies, 260);
    let m = mode(|file| file["crowd_target"] = 100.into());
    assert_eq!(horde("normal", Some(m)).caps().enemies, 130);
}

// --- The stage override -----------------------------------------------------------------------

#[test]
fn a_stage_override_is_one_to_the_most_any_target_can_be() {
    use stella_rain_core::parsed_stage_hash::parsed_stage_hash;
    use stella_rain_core::validate::{self, Error};
    let with = |target: u64| {
        let mut v = serde_json::to_value(crowd_stage(1, None)).unwrap();
        v["crowd_target"] = target.into();
        validate::parse_and_validate(v.to_string().as_bytes())
    };
    assert!(with(1).is_ok() && with(400).is_ok());
    for bad in [0, 401] {
        match with(bad) {
            Err(Error::Invalid(issues)) => {
                let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
                assert_eq!(paths, ["$.crowd_target"], "{bad}");
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
    // A stage without the field hashes as it did: the field is not written when absent.
    let plain = crowd_stage(1, None);
    let bytes = serde_json::to_vec(&plain).unwrap();
    assert!(!String::from_utf8(bytes).unwrap().contains("crowd_target"));
    assert_ne!(
        parsed_stage_hash(&plain),
        parsed_stage_hash(&crowd_stage(1, Some(100)))
    );
}
