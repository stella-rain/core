use super::*;
use crate::version::SIM_VERSION;

fn entry(id: &str, sim_version: u32) -> IntroducedId {
    IntroducedId {
        id: id.into(),
        sim_version,
    }
}

/// The issues `check_ids` raises for `listed` (each ID also an ID of the file).
fn run(listed: &[IntroducedId], sim_version: u32, released: &[IntroducedId]) -> Vec<Issue> {
    let file_ids: Vec<ContentId> = listed.iter().map(|e| e.id.clone()).collect();
    let ids: Vec<(String, &ContentId)> = file_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (format!("$.rewards.rewards[{i}].id"), id))
        .collect();
    let mut issues = Vec::new();
    check_ids(
        &mut issues,
        "$.introduced.rewards",
        listed,
        &ids,
        sim_version,
        released,
    );
    issues
}

#[test]
fn a_new_id_recorded_below_the_pack_is_refused() {
    let issues = run(&[entry("a", 0)], 1, &[]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "$.introduced.rewards[0].sim_version");
    assert_eq!(
        issues[0].message,
        "a new ID is introduced in the pack's sim_version 1, not 0"
    );
}

#[test]
fn a_released_id_keeps_its_version() {
    assert!(run(&[entry("a", 0)], 1, &[entry("a", 0)]).is_empty());
}

#[test]
fn a_new_id_at_the_pack_version_passes() {
    assert!(run(&[entry("a", 1)], 1, &[]).is_empty());
    assert!(run(&[entry("a", 0)], 0, &[]).is_empty());
}

#[test]
fn a_newer_entry_gets_only_the_newer_issue() {
    let issues = run(&[entry("a", 2)], 1, &[]);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("newer than the pack's 1"));
}

#[test]
fn only_the_new_entry_among_released_ones_is_refused() {
    let listed = [entry("r", 0), entry("n", 1), entry("m", 0)];
    let issues = run(&listed, 1, &[entry("r", 0)]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "$.introduced.rewards[2].sim_version");
}

#[test]
fn the_baseline_is_the_bundled_pack_and_holds_all_its_ids() {
    let file: PackFile = serde_json::from_slice(Pack::BUNDLED).expect("parses");
    assert_eq!(released(), &file.introduced);
    assert!(!released().rewards.is_empty());
    let _ = Pack::bundled();
}

#[test]
fn a_released_id_keeps_its_version_and_a_new_one_gets_the_pack_s() {
    let (a, b): (ContentId, ContentId) = ("a".into(), "b".into());
    let got = stamp(&[entry("a", 1)], vec![&a, &b], 2);
    assert_eq!(got, [entry("a", 1), entry("b", 2)]);
}

#[test]
fn the_bundled_content_exports_the_table_it_was_released_with() {
    let table = introduced_at(RunContent::bundled(), SIM_VERSION + 5);
    assert_eq!(table, *released());
}

#[test]
fn released_expertise_is_per_weapon() {
    let weapon = released().expertise[0].weapon.clone();
    assert!(!released_expertise(&weapon).is_empty());
    assert!(released_expertise(&"no_such_weapon".into()).is_empty());
}
