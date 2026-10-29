//! `progression::settle`, `open_difficulties` and `starting_unlocks` (CS1c): a run's Rune Shards,
//! the difficulties a clear opens and a new save's unlocks come from the run's content files.
//! "Today" is what the app computes at c3d1c43 (`tally.rs`, `setup.rs`, `migrations.rs`).

use proptest::prelude::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::ContentId;
use stella_rain_core::pack::Pack;
use stella_rain_core::progression::{
    Bank, RunOutcome, RunResult, Settlement, StartingSave, Unlock, UnlockKind, open_difficulties,
    settle, starting_unlocks,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CommandKind, RunReplay};
use stella_rain_core::stardust::MAX_BANK_CAP;
use stella_rain_core::validate::limits::MAX_RUN_STARDUST;

fn id(s: &str) -> ContentId {
    ContentId(s.to_owned())
}

fn ids(list: &[&str]) -> Vec<ContentId> {
    list.iter().map(|s| id(s)).collect()
}

fn run(mode: &str, difficulty: &str, outcome: RunOutcome, stardust_tenths: u32) -> RunResult {
    RunResult {
        mode: id(mode),
        difficulty: id(difficulty),
        outcome,
        stardust_tenths,
    }
}

/// The bundled pack with `edit` applied to its JSON: content B.
fn pack_with(edit: impl FnOnce(&mut Value)) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    edit(&mut v);
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the edited pack loads")
}

fn difficulty_mut<'a>(v: &'a mut Value, name: &str) -> &'a mut Value {
    v["difficulties"]["difficulties"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["id"] == name)
        .unwrap()
}

fn settle_bundled(run: &RunResult) -> Settlement {
    settle(RunContent::bundled(), run, &Bank::default())
}

#[test]
fn a_clear_pays_the_rune_shards_of_its_difficulty() {
    // tally.rs:208-216, tests.rs:2108-2114.
    for (difficulty, shards) in [("easy", 10), ("normal", 15), ("hard", 20), ("rainy", 30)] {
        let s = settle_bundled(&run("trial", difficulty, RunOutcome::Cleared, 125));
        assert_eq!(s.rune_shards, shards, "{difficulty}");
    }
    // A fail pays none (tests.rs:2122).
    let s = settle_bundled(&run("trial", "normal", RunOutcome::Failed, 40));
    assert_eq!(s.rune_shards, 0);
    assert!(s.opened.is_empty());
    // A difficulty the content does not hold pays none and opens none.
    let s = settle_bundled(&run("trial", "no_such", RunOutcome::Cleared, 125));
    assert_eq!(s.rune_shards, 0);
    assert!(s.opened.is_empty());
}

#[test]
fn a_clear_pays_its_stardust_whole() {
    // tests.rs:2115.
    let s = settle_bundled(&run("trial", "normal", RunOutcome::Cleared, 125));
    assert_eq!(s.paid_tenths, 125);
    assert_eq!(s.bank_paid_tenths, 0);
    assert_eq!(s.stardust_tenths, 125);
    assert_eq!(s.banked_tenths, 0);
    assert_eq!(s.lost_tenths, 0);
    assert_eq!(s.bank_after.get(&id("trial"), &id("normal")), 0);
    // A stage without a factor earns nothing (tests.rs:2130).
    let s = settle_bundled(&run("trial", "normal", RunOutcome::Cleared, 0));
    assert_eq!(
        (s.paid_tenths, s.bank_paid_tenths, s.stardust_tenths),
        (0, 0, 0)
    );
    assert_eq!((s.banked_tenths, s.lost_tenths), (0, 0));
}

/// The app's rule (setup.rs:98-104, OPEN_DIFFICULTIES = 2 at :200), copied here to compare
/// against: the first two of the file are open, each later one when the one before it is cleared.
fn app_rule(order: &[ContentId], cleared: &[ContentId]) -> Vec<ContentId> {
    order
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < 2 || cleared.contains(&order[i - 1]))
        .map(|(_, d)| d.clone())
        .collect()
}

#[test]
fn difficulties_open_as_the_app_opens_them_today() {
    // tests.rs:1785-1796.
    let content = RunContent::bundled();
    assert_eq!(open_difficulties(content, &[]), ids(&["easy", "normal"]));
    assert_eq!(
        open_difficulties(content, &ids(&["easy"])),
        ids(&["easy", "normal"])
    );
    assert_eq!(
        open_difficulties(content, &ids(&["normal"])),
        ids(&["easy", "normal", "hard"])
    );
    assert_eq!(
        open_difficulties(content, &ids(&["normal", "hard"])),
        ids(&["easy", "normal", "hard", "rainy"])
    );
    let order = ids(&["easy", "normal", "hard", "rainy"]);
    for subset in 0..16u32 {
        let cleared: Vec<ContentId> = order
            .iter()
            .enumerate()
            .filter(|(i, _)| subset & (1 << i) != 0)
            .map(|(_, d)| d.clone())
            .collect();
        assert_eq!(
            open_difficulties(content, &cleared),
            app_rule(&order, &cleared),
            "cleared {cleared:?}"
        );
    }
}

#[test]
fn a_clear_names_the_difficulties_it_opens() {
    for (cleared, opened) in [
        ("normal", &["hard"][..]),
        ("hard", &["rainy"]),
        ("easy", &[]),
        ("rainy", &[]),
    ] {
        let s = settle_bundled(&run("trial", cleared, RunOutcome::Cleared, 0));
        assert_eq!(s.opened, ids(opened), "{cleared}");
    }
    let s = settle_bundled(&run("trial", "normal", RunOutcome::Failed, 0));
    assert!(s.opened.is_empty());
}

#[test]
fn the_shards_and_the_opening_are_read_from_the_content() {
    let pack = pack_with(|v| difficulty_mut(v, "normal")["rune_shards"] = json!(17));
    let s = settle(
        pack.content(),
        &run("trial", "normal", RunOutcome::Cleared, 0),
        &Bank::default(),
    );
    assert_eq!(s.rune_shards, 17);

    let pack = pack_with(|v| difficulty_mut(v, "hard")["opens_after"] = json!("easy"));
    let content = pack.content();
    assert!(open_difficulties(content, &ids(&["easy"])).contains(&id("hard")));
    let s = settle(
        content,
        &run("trial", "easy", RunOutcome::Cleared, 0),
        &Bank::default(),
    );
    assert_eq!(s.opened, ids(&["hard"]));
}

#[test]
fn a_new_save_starts_with_the_unlocks_the_app_seeds_today() {
    // migrations.rs:43-49, preset_screen.gd:19-20, setup.rs:36.
    let save = starting_unlocks(RunContent::bundled()).expect("the bundled pack has them");
    let unlock = |kind, name: &str| Unlock { kind, id: id(name) };
    let expected = vec![
        unlock(UnlockKind::Weapon, "giant_sword"),
        unlock(UnlockKind::PerkSet, "duelist"),
        unlock(UnlockKind::StageSkill, "nova"),
        unlock(UnlockKind::Rune, "explosion"),
        unlock(UnlockKind::Rune, "bleed"),
        unlock(UnlockKind::Rune, "boomerang"),
    ];
    assert_eq!(
        save,
        StartingSave {
            unlocks: expected,
            free_weapon: id("giant_sword"),
            free_perk_set: id("duelist"),
        }
    );
    assert!(
        save.unlocks
            .contains(&unlock(UnlockKind::Weapon, "giant_sword"))
    );
    assert!(
        save.unlocks
            .contains(&unlock(UnlockKind::PerkSet, "duelist"))
    );

    let pack = pack_with(|v| {
        v.as_object_mut().unwrap().remove("starting_unlocks");
    });
    assert_eq!(starting_unlocks(pack.content()), None);
}

#[test]
fn a_result_exists_only_for_an_ended_run() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/current/runs/trial/run.replay");
    let replay = RunReplay::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let mut engine = Engine::start_with(&replay.setup, RunContent::bundled()).unwrap();
    assert_eq!(RunResult::from_run(&replay.setup, &engine), None);

    let commands = replay.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    for input in replay.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            match c.kind {
                CommandKind::Choose => engine.choose(c.index).unwrap(),
            }
        }
        engine.step(input);
    }
    let result = RunResult::from_run(&replay.setup, &engine).expect("the run ended");
    let outcome = match engine.outcome() {
        Outcome::Cleared => RunOutcome::Cleared,
        Outcome::Failed => RunOutcome::Failed,
        Outcome::Running => panic!("the corpus run did not end"),
    };
    assert_eq!(result.mode, id("trial"));
    assert_eq!(result.difficulty, id("normal"));
    assert_eq!(result.outcome, outcome);
    assert_eq!(result.stardust_tenths, engine.stardust());
}

// --- The Stardust split and the bank (ADR-101, ADR-SAVE-4 point 3) -----------------------------

fn bank_of(entries: &[(&str, &str, u64)]) -> Bank {
    Bank::new(
        entries
            .iter()
            .map(|(m, d, v)| ((id(m), id(d)), *v))
            .collect(),
    )
}

fn stored(bank: &Bank, mode: &str, difficulty: &str) -> u64 {
    bank.get(&id(mode), &id(difficulty))
}

/// The bundled pack with the trial's Stardust rules replaced.
fn trial_rules(bank: bool, pay_pct: u32, bank_pct: u32, bank_cap: u32) -> Pack {
    pack_with(|v| {
        let modes = v["stardust"]["modes"].as_array_mut().unwrap();
        let trial = modes.iter_mut().find(|m| m["id"] == "trial").unwrap();
        *trial = json!({ "id": "trial", "bank": bank, "pay_pct": pay_pct,
                         "bank_pct": bank_pct, "bank_cap": bank_cap });
    })
}

#[test]
fn a_fail_pays_half_and_banks_half() {
    let content = RunContent::bundled();
    let fail = |e| run("trial", "normal", RunOutcome::Failed, e);
    let s = settle(content, &fail(40), &Bank::default());
    assert_eq!((s.paid_tenths, s.banked_tenths, s.lost_tenths), (20, 20, 0));
    assert_eq!(s.stardust_tenths, 20);
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 20);
    // The bank share rounds up, the pay down.
    let s = settle(content, &fail(41), &Bank::default());
    assert_eq!((s.paid_tenths, s.banked_tenths, s.lost_tenths), (20, 21, 0));
    // Two fails, then the clear that pays the bank.
    let first = settle(content, &fail(40), &Bank::default());
    let second = settle(content, &fail(40), &first.bank_after);
    assert_eq!(stored(&second.bank_after, "trial", "normal"), 40);
    let clear = run("trial", "normal", RunOutcome::Cleared, 125);
    let s = settle(content, &clear, &second.bank_after);
    assert_eq!(s.bank_paid_tenths, 40);
    assert_eq!(s.paid_tenths, 125);
    assert_eq!(s.stardust_tenths, 165);
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 0);
}

#[test]
fn the_bank_belongs_to_its_mode_and_difficulty() {
    let content = RunContent::bundled();
    let bank = bank_of(&[("trial", "hard", 50), ("time_limit", "normal", 30)]);
    let s = settle(
        content,
        &run("trial", "normal", RunOutcome::Cleared, 125),
        &bank,
    );
    assert_eq!(s.bank_paid_tenths, 0);
    assert_eq!(stored(&s.bank_after, "trial", "hard"), 50);
    assert_eq!(stored(&s.bank_after, "time_limit", "normal"), 30);
    let s = settle(
        content,
        &run("trial", "hard", RunOutcome::Cleared, 125),
        &bank,
    );
    assert_eq!(s.bank_paid_tenths, 50);
    assert_eq!(stored(&s.bank_after, "trial", "hard"), 0);
    assert_eq!(stored(&s.bank_after, "time_limit", "normal"), 30);
}

#[test]
fn the_bank_stops_at_its_cap_even_when_a_save_holds_more() {
    // bank_cap 1000 whole Stardust: 10,000 tenths.
    let content = RunContent::bundled();
    let fail = run("trial", "normal", RunOutcome::Failed, 40);
    let s = settle(content, &fail, &bank_of(&[("trial", "normal", 9_990)]));
    assert_eq!(
        (s.paid_tenths, s.banked_tenths, s.lost_tenths),
        (20, 10, 10)
    );
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 10_000);

    let over = bank_of(&[("trial", "normal", 20_000)]);
    let clear = run("trial", "normal", RunOutcome::Cleared, 125);
    assert_eq!(settle(content, &clear, &over).bank_paid_tenths, 10_000);
    let s = settle(content, &fail, &over);
    assert_eq!((s.banked_tenths, s.lost_tenths), (0, 20));
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 10_000);
}

#[test]
fn with_the_bank_off_a_fail_banks_nothing_and_a_clear_pays_no_bank() {
    let pack = trial_rules(false, 50, 50, 1000);
    let bank = bank_of(&[("trial", "normal", 30)]);
    let s = settle(
        pack.content(),
        &run("trial", "normal", RunOutcome::Failed, 40),
        &bank,
    );
    assert_eq!((s.paid_tenths, s.banked_tenths, s.lost_tenths), (20, 0, 20));
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 30);
    let s = settle(
        pack.content(),
        &run("trial", "normal", RunOutcome::Cleared, 125),
        &bank,
    );
    assert_eq!((s.paid_tenths, s.bank_paid_tenths), (125, 0));
    assert_eq!(stored(&s.bank_after, "trial", "normal"), 30);
}

#[test]
fn todays_fail_is_a_pack_with_everything_paid() {
    // tests.rs:2123: a fail of 40 pays 40.
    let pack = trial_rules(false, 100, 0, 1000);
    let s = settle(
        pack.content(),
        &run("trial", "normal", RunOutcome::Failed, 40),
        &Bank::default(),
    );
    assert_eq!((s.stardust_tenths, s.banked_tenths), (40, 0));
    // A mode with no entry (tutorial) pays all and banks nothing, as the app does now.
    let bank = bank_of(&[("trial", "normal", 7)]);
    let s = settle(
        RunContent::bundled(),
        &run("tutorial", "normal", RunOutcome::Failed, 40),
        &bank,
    );
    assert_eq!((s.paid_tenths, s.banked_tenths, s.lost_tenths), (40, 0, 0));
    assert_eq!(s.bank_after, bank);
}

#[test]
fn the_split_is_read_from_the_content() {
    let pack = trial_rules(true, 30, 70, 1000);
    let s = settle(
        pack.content(),
        &run("trial", "normal", RunOutcome::Failed, 40),
        &Bank::default(),
    );
    assert_eq!((s.paid_tenths, s.banked_tenths), (12, 28));
}

#[test]
fn stardust_tenths_above_max_is_clamped_to_max_run_stardust() {
    let content = RunContent::bundled();
    let over = MAX_RUN_STARDUST.saturating_add(500);
    let clear = settle(
        content,
        &run("trial", "normal", RunOutcome::Cleared, over),
        &Bank::default(),
    );
    assert_eq!(clear.paid_tenths, u64::from(MAX_RUN_STARDUST));
    assert_eq!(clear.stardust_tenths, u64::from(MAX_RUN_STARDUST));

    let fail = settle(
        content,
        &run("trial", "normal", RunOutcome::Failed, over),
        &Bank::default(),
    );
    let max = u64::from(MAX_RUN_STARDUST);
    assert_eq!(fail.paid_tenths, max / 2);
    // Bundled trial has bank_cap = 1000 Stardust (10,000 tenths).
    assert_eq!(fail.banked_tenths, 10_000);
    assert_eq!(fail.lost_tenths, max / 2 - 10_000);
    assert_eq!(
        fail.paid_tenths + fail.banked_tenths + fail.lost_tenths,
        max
    );
}

#[test]
fn an_unknown_difficulty_returns_zero_pay_and_leaves_bank_unchanged() {
    let content = RunContent::bundled();
    let bank = bank_of(&[("trial", "no_such", 50)]);
    let clear = settle(
        content,
        &run("trial", "no_such", RunOutcome::Cleared, 125),
        &bank,
    );
    assert_eq!(
        (
            clear.paid_tenths,
            clear.bank_paid_tenths,
            clear.stardust_tenths,
            clear.banked_tenths,
            clear.lost_tenths
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(clear.bank_after, bank);
    assert_eq!(clear.rune_shards, 0);
    assert!(clear.opened.is_empty());

    let fail = settle(
        content,
        &run("trial", "no_such", RunOutcome::Failed, 100),
        &bank,
    );
    assert_eq!(
        (
            fail.paid_tenths,
            fail.bank_paid_tenths,
            fail.stardust_tenths,
            fail.banked_tenths,
            fail.lost_tenths
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(fail.bank_after, bank);
    assert_eq!(fail.rune_shards, 0);
    assert!(fail.opened.is_empty());
}

proptest! {
    /// Nothing is made or lost by the split; the bank never passes its cap.
    #[test]
    fn a_settlement_keeps_the_books(
        earned in 0..=MAX_RUN_STARDUST,
        pay_pct in 0u32..=100,
        bank_share in 0u32..=100,
        bank_cap in 0..=MAX_BANK_CAP,
        bank_on in any::<bool>(),
        before in any::<u64>(),
    ) {
        let bank_pct = bank_share.min(100 - pay_pct);
        let pack = trial_rules(bank_on, pay_pct, bank_pct, bank_cap);
        let cap = u64::from(bank_cap) * 10;
        let bank = bank_of(&[("trial", "normal", before)]);
        let fail = settle(
            pack.content(),
            &run("trial", "normal", RunOutcome::Failed, earned),
            &bank,
        );
        let e = u64::from(earned);
        prop_assert_eq!(fail.paid_tenths + fail.banked_tenths + fail.lost_tenths, e);
        let after = stored(&fail.bank_after, "trial", "normal");
        if bank_on {
            prop_assert!(after <= cap);
        } else {
            // A bank that is off stays as the save holds it.
            prop_assert_eq!(after, before);
        }
        let clear = settle(
            pack.content(),
            &run("trial", "normal", RunOutcome::Cleared, earned),
            &bank,
        );
        prop_assert_eq!(clear.paid_tenths, e);
        prop_assert!(clear.bank_paid_tenths <= cap);
    }
}
