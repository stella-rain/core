//! The two Hammer run cases, `hammer_titan` and `hammer_momentum` (ADR-SIM-7, ADR-SIM-9): the
//! solo run corpus plays `giant_hammer` with each of two styles locked in the setup, and these
//! promises read the picks, the capstone and their effects from the replay and its re-simulated
//! events, never from the seed. The promise is `hammer_miss`: the first predicate a run misses,
//! as a message; `promise` in `corpus_seeds.rs` and the tests below use it.

use crate::common::run::HAMMER_TICKS;
use crate::common::run_steps::{self, RunSteps, Step};
use crate::corpus::{case_content, runs_dir};
use std::fs;
use std::path::Path;
use stella_rain_core::engine::Outcome;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::EquipAction;
use stella_rain_core::run_replay::{self, RunReplay};
use stella_rain_core::snapshot::ExpertiseOfferKind;

/// What a Hammer run did, read by re-simulating it.
#[derive(Default)]
pub(crate) struct HammerReach {
    /// Each `TreePick`: the offer's kind, the id at the taken option and the tick it was taken at.
    pub picks: Vec<(ExpertiseOfferKind, String, u32)>,
    /// Each giant trigger that ended: its start tick, its length and its end tick (a restart while
    /// it runs drops the open one).
    pub triggers: Vec<(u32, u32, u32)>,
    /// The ticks a hold was released at, and whether a trigger was running then.
    pub holds: Vec<(u32, bool)>,
    /// The tempo's most stacks, once the style shows a tempo.
    pub max_stacks: Option<u32>,
    /// The ticks at which a swing raised the tempo, with the stacks it reached.
    pub raised: Vec<(u32, u32)>,
    /// Each combo reset: its tick and the stacks before and after it.
    pub resets: Vec<(u32, u32, u32)>,
}

/// Re-simulates the case in `case` (a verified run) and reads what it did. Panics when the run
/// does not verify or is not a run: `hammer_miss` checks that first.
pub(crate) fn hammer_reach_at(case: &Path) -> HammerReach {
    let content = case_content(case).unwrap();
    let replay = RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = run_steps::start(&replay, &content);
    let mut r = HammerReach::default();
    let mut open: Option<(u32, u32)> = None;
    for step in RunSteps::of(&replay).steps() {
        let before = engine.snapshot().players[0].tempo.map_or(0, |t| t.stacks);
        if let Step::Actions(actions) = step
            && let Some(EquipAction::TreePick { option }) = actions[0]
        {
            let ready = engine.snapshot().players[0]
                .tree_ready
                .clone()
                .expect("a tree pick is made at an offer");
            let id = ready.options[option as usize].as_str().to_string();
            r.picks.push((ready.kind, id, engine.tick()));
        }
        run_steps::apply(&mut engine, step);
        if !matches!(step, Step::Tick(_)) {
            continue;
        }
        let tick = engine.tick();
        let s = engine.snapshot();
        let after = s.players[0].tempo.map_or(0, |t| t.stacks);
        if let Some(t) = s.players[0].tempo {
            r.max_stacks.get_or_insert(t.max_stacks);
        }
        for e in engine.events() {
            match e {
                DomainEvent::TriggerStarted { ticks, .. } => open = Some((tick, *ticks)),
                DomainEvent::TriggerEnded { .. } => {
                    if let Some((start, len)) = open.take() {
                        r.triggers.push((start, len, tick));
                    }
                }
                DomainEvent::HoldReleased { .. } => r.holds.push((tick, open.is_some())),
                DomainEvent::TempoRaised { stacks, .. } => r.raised.push((tick, *stacks)),
                DomainEvent::ComboReset => r.resets.push((tick, before, after)),
                _ => {}
            }
        }
    }
    r
}

/// The first predicate the case in `case` misses of the promise of the Hammer style `style`
/// (`titan_trigger` or `momentum`), or `None`. Each predicate reads the replay and the
/// re-simulated state, never the seed; the weapon is the first.
pub(crate) fn hammer_miss(case: &Path, style: &str) -> Option<String> {
    let bytes = fs::read(case.join("run.replay")).unwrap();
    let replay = RunReplay::from_bytes(&bytes).unwrap();
    if replay.setup.weapon.as_str() != "giant_hammer" {
        return Some(format!(
            "the weapon is {}, not giant_hammer",
            replay.setup.weapon.as_str()
        ));
    }
    let locked: Option<Vec<&str>> = replay
        .setup
        .unlocked_styles
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());
    if locked.as_deref() != Some(&[style][..]) {
        return Some(format!(
            "the styles unlocked are {locked:?}, not [{style:?}]"
        ));
    }
    let content = match case_content(case) {
        Ok(c) => c,
        Err(e) => return Some(format!("the case has no content: {e}")),
    };
    let v = match run_replay::verify_run_with(&bytes, &content) {
        Ok(v) => v,
        Err(e) => return Some(format!("the run does not verify: {e:?}")),
    };
    if v.outcome != Outcome::Running || v.ticks != HAMMER_TICKS {
        return Some(format!(
            "the run ends {:?} at {}, not Running at {HAMMER_TICKS}",
            v.outcome, v.ticks
        ));
    }
    let r = hammer_reach_at(case);
    let want = [
        (ExpertiseOfferKind::Style, style.to_string()),
        (ExpertiseOfferKind::Upgrade, format!("{style}_up_1")),
        (ExpertiseOfferKind::Upgrade, format!("{style}_up_2")),
        (ExpertiseOfferKind::Capstone, format!("{style}_cap")),
    ];
    let got: Vec<_> = r.picks.iter().map(|p| (p.0, p.1.clone())).collect();
    if got != want {
        return Some(format!("the picks are {got:?}, not {want:?}"));
    }
    let (up2, cap) = (r.picks[2].2, r.picks[3].2);
    match style {
        "titan_trigger" => titan_miss(&r, up2, cap),
        "momentum" => momentum_miss(&r, cap),
        other => panic!("no Hammer promise for the style `{other}`"),
    }
}

/// Titan: a trigger started at or after the capstone that ran its whole length out (the end ring
/// then plays: inferred from `trigger_tick`, not read), and a hold released under a running
/// trigger after `titan_trigger_up_2`.
fn titan_miss(r: &HammerReach, up2: u32, cap: u32) -> Option<String> {
    if !r
        .triggers
        .iter()
        .any(|&(s, len, e)| s >= cap && e == s + len)
    {
        return Some(format!(
            "no trigger started after the capstone (tick {cap}) ran out: {:?}",
            r.triggers
        ));
    }
    if !r.holds.iter().any(|&(t, running)| t >= up2 && running) {
        return Some(format!(
            "no hold is released under a running trigger after up_2 (tick {up2}): {:?}",
            r.holds
        ));
    }
    None
}

/// Momentum: full stacks reached after the capstone, a combo reset that keeps them after it and
/// none before it, and a hold released.
fn momentum_miss(r: &HammerReach, cap: u32) -> Option<String> {
    let Some(max) = r.max_stacks else {
        return Some("the style shows no tempo".into());
    };
    if !r.raised.iter().any(|&(t, n)| t >= cap && n == max) {
        return Some(format!(
            "the tempo never reaches {max} stacks after the capstone (tick {cap}): {:?}",
            r.raised
        ));
    }
    let carries = |b: u32, a: u32| b == max && a == max;
    if !r.resets.iter().any(|&(t, b, a)| t >= cap && carries(b, a)) {
        return Some(format!(
            "no combo reset keeps full stacks after the capstone (tick {cap})"
        ));
    }
    if let Some(&(t, ..)) = r.resets.iter().find(|&&(t, b, a)| t < cap && carries(b, a)) {
        return Some(format!(
            "a combo reset keeps full stacks at tick {t}, before the capstone (tick {cap})"
        ));
    }
    if r.holds.is_empty() {
        return Some("no hold is released".into());
    }
    None
}

/// The promise as a panic, for the corpus tests and the seed search.
pub(crate) fn assert_hammer_titan(case: &Path) {
    if let Some(miss) = hammer_miss(case, "titan_trigger") {
        panic!("{miss}");
    }
}

pub(crate) fn assert_hammer_momentum(case: &Path) {
    if let Some(miss) = hammer_miss(case, "momentum") {
        panic!("{miss}");
    }
}

fn case(name: &str) -> std::path::PathBuf {
    let dir = runs_dir().join(name);
    assert!(dir.is_dir(), "the case `{name}` is not in the corpus");
    dir
}

#[test]
fn the_hammer_cases_are_what_their_names_say() {
    let (titan, momentum) = (case("hammer_titan"), case("hammer_momentum"));
    assert_hammer_titan(&titan);
    assert_hammer_momentum(&momentum);
}

#[test]
fn the_hammer_promises_separate_the_styles() {
    let (titan, momentum) = (case("hammer_titan"), case("hammer_momentum"));
    let miss = hammer_miss(&momentum, "titan_trigger").expect("momentum is not titan_trigger");
    assert!(miss.contains("momentum"), "{miss}");
    let miss = hammer_miss(&titan, "momentum").expect("titan_trigger is not momentum");
    assert!(miss.contains("titan_trigger"), "{miss}");
    assert_eq!(hammer_miss(&titan, "titan_trigger"), None);
    assert_eq!(hammer_miss(&momentum, "momentum"), None);
}

#[test]
fn the_hammer_promises_refuse_a_sword_run() {
    // `time_limit` is a giant_sword run, Failed at 14,924: the weapon is the first predicate.
    let sword = case("time_limit");
    for style in ["titan_trigger", "momentum"] {
        let miss = hammer_miss(&sword, style).expect("a sword run is no Hammer run");
        assert!(miss.contains("giant_hammer"), "{miss}");
        assert!(miss.contains("giant_sword"), "{miss}");
    }
}
