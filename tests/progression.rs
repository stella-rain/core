//! `progression::settle`, `open_difficulties` and `starting_unlocks` (CS1c): a run's Rune Shards,
//! the difficulties a clear opens and a new save's unlocks come from the run's content files.
//! "Today" is what the app computes at c3d1c43 (`tally.rs`, `setup.rs`, `migrations.rs`).

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
        .join("tests/corpus/runs/v0/trial/run.replay");
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
