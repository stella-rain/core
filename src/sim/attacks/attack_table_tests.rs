//! Tests of the attack table: the phase timing against its reference oracle.

use super::*;
use crate::attack::Offset;
use crate::run_content::RunContent;
use crate::sim::state::{Aim, Dir};
use crate::sim::world::World;

// The two functions below are `faster` and `wider` as they were before attacks were compiled
// into the world's table, kept as the oracle for `phase`.
fn reference_faster(phases: &[AttackPhase], pct: u32) -> Vec<AttackPhase> {
    let per = 100 + u64::from(pct);
    let total: u64 = phases.iter().map(|p| u64::from(p.ticks)).sum();
    let target = ((total * 100 + per / 2) / per).max(phases.len() as u64);
    let mut out: Vec<AttackPhase> = phases.to_vec();
    let mut rests: Vec<(u64, usize)> = Vec::with_capacity(phases.len());
    let mut sum = 0u64;
    for (i, p) in phases.iter().enumerate() {
        let exact = u64::from(p.ticks) * 100;
        let ticks = (exact / per).max(1);
        rests.push((exact.saturating_sub(ticks * per), i));
        out[i].ticks = u32::try_from(ticks).unwrap_or(u32::MAX);
        sum += ticks;
    }
    rests.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, i) in rests
        .iter()
        .take(usize::try_from(target.saturating_sub(sum)).unwrap_or(usize::MAX))
    {
        out[i].ticks = out[i].ticks.saturating_add(1);
    }
    out
}

fn reference_wider(phases: &[AttackPhase], pct: u32) -> Vec<AttackPhase> {
    let per = 100 + i64::from(pct.min(MAX_RANGE_PCT));
    let grow = |v: &mut Fx| {
        v.0 = i32::try_from(i64::from(v.0) * per / 100).unwrap_or(if v.0 < 0 {
            i32::MIN
        } else {
            i32::MAX
        });
    };
    let mut out = phases.to_vec();
    for p in &mut out {
        match &mut p.hit_box {
            Some(HitBox::Circle { offset, radius, .. } | HitBox::Arc { offset, radius, .. }) => {
                grow(radius);
                grow(&mut offset.forward);
                grow(&mut offset.right);
            }
            Some(HitBox::Line {
                offset,
                length,
                width,
                ..
            }) => {
                grow(length);
                grow(width);
                grow(&mut offset.forward);
                grow(&mut offset.right);
            }
            None => {}
        }
    }
    out
}

fn phase_of(kind: PhaseKind, ticks: u32, hit_box: Option<HitBox>) -> AttackPhase {
    AttackPhase {
        kind,
        ticks,
        hit_box,
        damage: None,
        damage_type: None,
        rehit_ticks: None,
        spawn: Vec::new(),
        move_pct: 100,
        breaks_charge: false,
        dash: None,
    }
}

fn offset(forward: i32, right: i32) -> Offset {
    Offset {
        forward: Fx(forward),
        right: Fx(right),
    }
}

/// A combo step of every hit box shape, with lengths that do not divide evenly.
fn combo_step() -> Vec<AttackPhase> {
    vec![
        phase_of(PhaseKind::WindUp, 7, None),
        phase_of(
            PhaseKind::Active,
            5,
            Some(HitBox::Circle {
                anchor: Default::default(),
                offset: offset(3_000, -1_000),
                radius: Fx(9_001),
            }),
        ),
        phase_of(
            PhaseKind::Active,
            4,
            Some(HitBox::Arc {
                anchor: Default::default(),
                offset: offset(0, 777),
                radius: Fx(20_000),
                half_angle: 5_000,
            }),
        ),
        phase_of(
            PhaseKind::Active,
            3,
            Some(HitBox::Line {
                anchor: Default::default(),
                offset: offset(-501, 2),
                angle: 100,
                length: Fx(30_001),
                width: Fx(7_777),
            }),
        ),
        phase_of(PhaseKind::Recovery, 11, None),
    ]
}

fn run_of(attack: AttackIndex, speed_pct: u32, width_pct: u32) -> AttackRun {
    AttackRun {
        owner: EntityId(1),
        slot: 0,
        attack,
        speed_pct,
        width_pct,
        phase: 0,
        tick: 0,
        facing: Dir::DOWN,
        aim: Aim::Fixed(Dir::DOWN),
        friendly: true,
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

#[test]
fn a_phase_read_through_the_table_is_what_faster_and_wider_made() {
    let attack = AttackRef::Inline(combo_step());
    let mut world = World::for_test(RunContent::bundled().trial().clone());
    let index = world.compile_attack(&attack).unwrap();
    // (attack speed, range): none, the case of the task, each alone, and the cap.
    let cases = [
        (0, 0),
        (30, 0),
        (0, 25),
        (30, 25),
        (30, 33),
        (100, 50),
        (1, 1),
        (250, MAX_RANGE_PCT),
    ];
    for (speed, range) in cases {
        let mut oracle = combo_step();
        if speed != 0 {
            oracle = reference_faster(&oracle, speed);
        }
        if range != 0 {
            oracle = reference_wider(&oracle, range);
        }
        let run = run_of(index, speed, range);
        for (i, expected) in oracle.iter().enumerate() {
            let got = phase(&world, &run, i);
            assert_eq!(got.ticks, expected.ticks, "ticks of {i} at {speed}/{range}");
            assert_eq!(
                got.hit_box(),
                expected.hit_box.as_ref(),
                "box of {i} at {speed}/{range}"
            );
            assert_eq!(got.kind, expected.kind);
            assert_eq!(got.move_pct, expected.move_pct);
        }
        assert_eq!(phase_count(&world, &run), oracle.len());
    }
}

#[test]
fn an_attack_compiles_once_and_a_preset_keeps_its_name() {
    let mut world = World::for_test(RunContent::bundled().trial().clone());
    let inline = AttackRef::Inline(combo_step());
    let a = world.compile_attack(&inline).unwrap();
    assert_eq!(world.compile_attack(&inline).unwrap(), a);
    assert_eq!(world.attacks[usize::from(a.0)].preset, None);
    let other = AttackRef::Inline(vec![phase_of(PhaseKind::Active, 2, None)]);
    assert_ne!(world.compile_attack(&other).unwrap(), a);
}
