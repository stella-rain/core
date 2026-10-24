use super::*;
use crate::attack::{AttackPhase, AttackRef, PhaseKind};
use crate::id::EntityId;

fn phase(move_pct: u8) -> AttackPhase {
    AttackPhase {
        kind: PhaseKind::Active,
        ticks: 5,
        hit_box: None,
        damage: None,
        damage_type: None,
        rehit_ticks: None,
        spawn: Vec::new(),
        move_pct,
        breaks_charge: false,
    }
}

fn world() -> World {
    World::for_test(RunContent::bundled().trial().clone())
}

/// A run of `owner` whose phases have these `move_pct`s, on its `phase`.
fn run(world: &mut World, owner: u32, phase_at: u32, pcts: &[u8]) -> AttackRun {
    let attack = AttackRef::Inline(pcts.iter().map(|p| phase(*p)).collect());
    AttackRun {
        owner: EntityId(owner),
        slot: 0,
        attack: world.compile_attack(&attack).unwrap(),
        speed_pct: 0,
        width_pct: 0,
        phase: phase_at,
        tick: 0,
        facing: Dir::DOWN,
        aim: Aim::Fixed(Dir::DOWN),
        friendly: false,
        ranged: false,
        base: 1,
        atk_up_pct: 0,
        damage_pct: 100,
        type_bonus_pct: None,
        age: 0,
        hits: Vec::new(),
        smash: None,
        at: None,
        throw: None,
        homing: None,
        spot: None,
    }
}

fn move_pct_of(world: &World, r: &AttackRun) -> i64 {
    i64::from(world.attacks[usize::from(r.attack.0)].phases[r.phase as usize].move_pct)
}

/// The scan the index replaces, kept as the oracle.
fn scan(world: &World, runs: &[AttackRun], owner: EntityId) -> i64 {
    runs.iter()
        .filter(|r| r.owner == owner)
        .map(|r| move_pct_of(world, r))
        .min()
        .unwrap_or(100)
}

fn assert_same(world: &World, index: &MoveIndex, runs: &[AttackRun]) {
    for id in 0..8 {
        assert_eq!(
            index.get(EntityId(id)),
            scan(world, runs, EntityId(id)),
            "owner {id}"
        );
    }
}

#[test]
fn an_owner_with_several_runs_gets_the_lowest_move_pct() {
    let mut w = world();
    let runs = vec![
        run(&mut w, 3, 0, &[80, 10]),
        run(&mut w, 5, 1, &[0, 100]),
        run(&mut w, 3, 1, &[100, 40]),
        run(&mut w, 3, 0, &[90]),
        run(&mut w, 1, 0, &[0]),
    ];
    let mut index = MoveIndex::default();
    index.rebuild(&w, &runs);
    assert_same(&w, &index, &runs);
    assert_eq!(index.get(EntityId(3)), 40);
    assert_eq!(index.get(EntityId(1)), 0);
    assert_eq!(index.get(EntityId(2)), 100);
}

#[test]
fn lowering_for_a_new_run_matches_a_rebuild() {
    let mut w = world();
    let mut runs = vec![run(&mut w, 4, 0, &[70]), run(&mut w, 2, 0, &[100])];
    let mut index = MoveIndex::default();
    index.rebuild(&w, &runs);
    for (owner, pcts) in [(4, [30u8]), (6, [100]), (2, [55]), (6, [0]), (0, [90])] {
        let r = run(&mut w, owner, 0, &pcts);
        index.lower(r.owner, move_pct_of(&w, &r));
        runs.push(r);
        assert_same(&w, &index, &runs);
    }
    let mut rebuilt = MoveIndex::default();
    rebuilt.rebuild(&w, &runs);
    assert_same(&w, &rebuilt, &runs);
}
