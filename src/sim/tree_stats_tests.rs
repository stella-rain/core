//! Tests for the two new stat effects and the tenths array a weapon tree's stat set is written
//! to (ADR-SIM-7, ADR-SIM-3): the type, the reads, and the state layout.

use super::levels::{self, attack_of};
use super::levels_tests::{hammer, progress, state_of, world_state};
use super::skill;
use super::state::PlayerIdx;
use crate::engine::Engine;
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;

fn set_progress(e: &mut Engine, f: impl FnOnce(&mut super::state::Progress)) {
    let (_, state) = world_state(e);
    f(state
        .players
        .career_mut(PlayerIdx::SOLO)
        .progress
        .as_mut()
        .expect("levels"));
}

/// T3: the combo's range is the player's own, the expertise's and the stat's whole percent.
#[test]
fn range_stat_widens_the_combo() {
    let mut e = hammer();
    let me = PlayerIdx::SOLO;
    assert_eq!(levels::range_pct(state_of(&e), me), 0);
    set_progress(&mut e, |p| p.totals[StatEffect::RangePct as usize] = 25);
    assert_eq!(levels::range_pct(state_of(&e), me), 25);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 0;
        p.set_tenths[StatEffect::RangePct as usize] = 250;
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 25);
    set_progress(&mut e, |p| {
        p.set_tenths[StatEffect::RangePct as usize] = 259
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 25, "one round down");
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 25;
        p.set_tenths[StatEffect::RangePct as usize] = 10;
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 26);
    // The sum of the sources: the player's own, the expertise's and the stat's.
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 20;
        p.set_tenths[StatEffect::RangePct as usize] = 0;
    });
    {
        let (_, s) = world_state(&mut e);
        s.players[me].range_pct = 50;
        s.players
            .career_mut(me)
            .expertise
            .as_mut()
            .expect("gauge")
            .range_pct = 10;
    }
    assert_eq!(levels::range_pct(state_of(&e), me), 80);
}

/// T4: the stat shortens the stage skill's cooldown, added to the reward's percent, and the
/// sum is held to the existing cap.
#[test]
fn skill_cooldown_stat_shortens_the_skill() {
    let mut e = hammer();
    let me = PlayerIdx::SOLO;
    let at = |e: &Engine| skill::cooldown_after(state_of(e), me, 1000);
    assert_eq!(at(&e), 1000);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 10
    });
    assert_eq!(at(&e), 900);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 60
    });
    assert_eq!(at(&e), 400);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 90
    });
    assert_eq!(at(&e), 400);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 10;
    });
    world_state(&mut e).1.players[me].skill_cooldown_pct = 20;
    assert_eq!(at(&e), 700);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 0;
        p.set_tenths[StatEffect::SkillCooldownPct as usize] = 100;
    });
    assert_eq!(at(&e), 700, "10.0 percent of the set reads as 10");
}

/// T6: an offers run never writes the new totals or the tenths.
#[test]
fn the_new_values_are_zero_in_an_offers_run() {
    let mut e = Engine::start(&RunSetup {
        mode: "time_limit".into(),
        build_mode: BuildMode::Offers,
        ..RunSetup::default()
    })
    .expect("starts");
    for _ in 0..600 {
        e.step(&[crate::input::Input::default(); crate::id::MAX_PLAYERS as usize]);
    }
    let p = progress(&e);
    assert_eq!((p.totals[11], p.totals[12]), (0, 0));
    assert!(p.set_tenths.iter().all(|t| *t == 0));
}

/// T7: with no set the tenths reads are today's numbers.
#[test]
fn a_set_free_read_is_todays_number() {
    let e = hammer();
    let mut p = progress(&e).clone();
    for w in 0..=100u32 {
        p.totals[StatEffect::AttackPct as usize] = w;
        for atk in [0u32, 1, 7, 99, 1000, 123_457, 4_000_000] {
            let old = u32::try_from(u64::from(atk) * u64::from(100 + w) / 100).unwrap();
            assert_eq!(attack_of(Some(&p), atk), old, "w {w} atk {atk}");
        }
        p.totals[StatEffect::RangePct as usize] = w;
        assert_eq!(levels::set_pct(&p, StatEffect::RangePct), w);
    }
    // A set value is exact on attack and rounds down once on the whole-percent reads.
    p.totals = [0; StatEffect::ALL.len()];
    p.set_tenths[StatEffect::AttackPct as usize] = 165;
    assert_eq!(attack_of(Some(&p), 1000), 1165);
    p.set_tenths[StatEffect::AttackSpeedPct as usize] = 35;
    assert_eq!(levels::set_pct(&p, StatEffect::AttackSpeedPct), 3);
}

// --- the weapon's stat sets -------------------------------------------------------------------

use super::expertise;
use super::levels_tests::{gauge, tree_pick};
use crate::expertise::Expertise;
use crate::input::Input;

const AP: usize = StatEffect::AttackPct as usize;
const AS: usize = StatEffect::AttackSpeedPct as usize;
const CD: usize = StatEffect::SkillCooldownPct as usize;
const RG: usize = StatEffect::RangePct as usize;

/// A time-limit equipment engine with the sword, whose trees are `edge`, `tempo` and `reach`.
fn sword() -> Engine {
    Engine::start(&RunSetup {
        mode: "time_limit".into(),
        build_mode: BuildMode::Equipment,
        weapon: "giant_sword".into(),
        ..RunSetup::default()
    })
    .expect("starts")
}

/// Raises player 0's weapon level to `level` through the rules' own level-up.
fn level_to(e: &mut Engine, level: u32) {
    let (world, state) = world_state(e);
    while state
        .players
        .career(PlayerIdx::SOLO)
        .expertise
        .as_ref()
        .expect("a gauge")
        .level
        < level
    {
        let g = state
            .players
            .career_mut(PlayerIdx::SOLO)
            .expertise
            .as_mut()
            .expect("a gauge");
        g.points = g.next;
        expertise::level_up(world, Expertise::bundled(), state, PlayerIdx::SOLO);
    }
}

fn tenths(e: &Engine) -> [u32; StatEffect::ALL.len()] {
    progress(e).set_tenths
}

/// S1: Kade's example: the tempo set at weapon level 5.
#[test]
fn the_tempo_set_at_lv5_is_kades_example() {
    let mut e = sword();
    level_to(&mut e, 5);
    tree_pick(&mut e, 1).expect("tempo");
    assert_eq!(gauge(&e).style, Some(1));
    let t = tenths(&e);
    assert_eq!((t[AP], t[AS], t[CD], t[RG]), (165, 500, 100, 250));
    let others = StatEffect::ALL
        .iter()
        .filter(|f| ![AP, AS, CD, RG].contains(&(**f as usize)))
        .all(|f| t[*f as usize] == 0);
    assert!(others);
    // The reads, with no other source in the totals.
    let mut p = progress(&e).clone();
    p.totals = [0; StatEffect::ALL.len()];
    assert_eq!(attack_of(Some(&p), 1000), 1165);
    assert_eq!(levels::set_pct(&p, StatEffect::AttackSpeedPct), 50);
    assert_eq!(levels::set_pct(&p, StatEffect::SkillCooldownPct), 10);
    assert_eq!(levels::set_pct(&p, StatEffect::RangePct), 25);
}

/// S2: a level-up recomputes the set and does not stack it; levels past 10 stop growing.
#[test]
fn a_level_up_recomputes_and_does_not_stack() {
    let mut e = sword();
    level_to(&mut e, 3);
    tree_pick(&mut e, 1).expect("tempo");
    for level in 3..=10u32 {
        level_to(&mut e, level);
        assert_eq!(tenths(&e)[AS], 100 * level, "level {level}");
        assert_eq!(tenths(&e)[AP], 33 * level, "level {level}");
    }
    for level in [11, 50, 100] {
        level_to(&mut e, level);
        assert_eq!((tenths(&e)[AS], tenths(&e)[AP]), (1000, 330), "{level}");
    }
    // The pure function: level 0 reads as Lv1.
    let at = |lv| expertise::tree_stats("giant_sword", Some("tempo"), lv);
    assert_eq!(
        (at(0)[AS], at(1)[AS], at(2)[AS], at(10)[AS]),
        (100, 100, 200, 1000)
    );
    assert_eq!(at(1000), at(10));
}

/// S3: the base set applies from tick 0, to every slot, a joiner too.
#[test]
fn the_base_set_applies_from_tick_0() {
    let mut e = sword();
    assert_eq!((tenths(&e)[AP], tenths(&e)[AS]), (10, 10));
    level_to(&mut e, 7);
    assert_eq!((tenths(&e)[AP], tenths(&e)[AS]), (70, 70));

    let party = Engine::start_party(
        &RunSetup {
            mode: "time_limit".into(),
            build_mode: BuildMode::Equipment,
            weapon: "giant_sword".into(),
            ..RunSetup::default()
        },
        3,
    )
    .expect("a party");
    for slot in 0..3 {
        let c = state_of(&party)
            .players
            .career(PlayerIdx::new(slot).expect("a slot"));
        let p = c.progress.as_ref().expect("levels");
        assert_eq!(
            (p.set_tenths[AP], p.set_tenths[AS]),
            (10, 10),
            "slot {slot}"
        );
    }

    let mut solo = sword();
    solo.join(PlayerIdx::new(1).expect("a slot"))
        .expect("joins");
    let p = state_of(&solo)
        .players
        .career(PlayerIdx::new(1).expect("a slot"))
        .progress
        .as_ref()
        .expect("levels");
    assert_eq!((p.set_tenths[AP], p.set_tenths[AS]), (10, 10));
}

/// S4: the tree pick replaces the base set.
#[test]
fn the_tree_pick_replaces_the_base_set() {
    let mut e = sword();
    level_to(&mut e, 5);
    assert_eq!((tenths(&e)[AP], tenths(&e)[AS]), (50, 50));
    tree_pick(&mut e, 1).expect("tempo");
    assert_eq!(tenths(&e)[AP], 165, "not 215");
    let mut e = sword();
    level_to(&mut e, 5);
    tree_pick(&mut e, 0).expect("edge");
    let t = tenths(&e);
    assert_eq!((t[AP], t[AS], t[CD], t[RG]), (250, 150, 50, 100));
}

/// S5: a recompute leaves every other source in `totals` as it was.
#[test]
fn other_sources_are_untouched_by_a_recompute() {
    let mut e = sword();
    set_progress(&mut e, |p| {
        p.totals[AP] = 10;
        p.totals[StatEffect::StardustPct as usize] = 5;
    });
    let read = |e: &Engine| attack_of(Some(progress(e)), 1000);
    assert_eq!(read(&e), 1101);
    level_to(&mut e, 5);
    assert_eq!(read(&e), 1150);
    tree_pick(&mut e, 1).expect("tempo");
    assert_eq!(read(&e), 1265);
    let p = progress(&e);
    assert_eq!(
        (p.totals[AP], p.totals[StatEffect::StardustPct as usize]),
        (10, 5)
    );
}

/// S6: a hit-point set moves the most hit points by whole units, and never below 1.
#[test]
fn max_hp_in_a_set_moves_max_hp_and_never_below_one() {
    let mut e = hammer();
    level_to(&mut e, 5);
    tree_pick(&mut e, 2).expect("bulwark");
    let hp = |e: &Engine| state_of(e).players[PlayerIdx::SOLO].max_hp;
    let at5 = hp(&e);
    level_to(&mut e, 6);
    assert_eq!(hp(&e), at5 + 4000, "Lv6 is 24000, Lv5 is 20000");
    assert_eq!(
        tenths(&e)[StatEffect::MaxHp as usize],
        0,
        "whole units live in totals"
    );

    // The writer with a larger `was`: the most hit points never go below 1.
    let (world, state) = world_state(&mut e);
    let (player, career) = state.players.both_mut(PlayerIdx::SOLO);
    player.max_hp = 100;
    player.hp = 100;
    let p = career.progress.as_mut().expect("levels");
    expertise::write_set(
        world,
        Expertise::bundled(),
        p,
        player,
        Some((Some(2), 10)),
        (Some(2), 1),
    );
    assert_eq!((player.max_hp, player.hp), (1, 1));
}

/// S7: a weapon or a style the table does not name has the empty set; a run with no levels
/// writes nothing and does not panic.
#[test]
fn an_unlisted_weapon_or_style_has_the_empty_set() {
    let zero = [0; StatEffect::ALL.len()];
    assert_eq!(expertise::tree_stats("nowhere", None, 5), zero);
    assert_eq!(expertise::tree_stats("nowhere", Some("tempo"), 5), zero);
    assert_eq!(
        expertise::tree_stats("giant_sword", Some("nowhere"), 5),
        zero
    );
    assert_ne!(expertise::tree_stats("giant_sword", None, 5), zero);

    let mut e = sword();
    set_progress(&mut e, |_| {});
    world_state(&mut e)
        .1
        .players
        .career_mut(PlayerIdx::SOLO)
        .progress = None;
    level_to(&mut e, 4);
    assert!(
        state_of(&e)
            .players
            .career(PlayerIdx::SOLO)
            .progress
            .is_none()
    );
}

/// A scripted run: levels crossed by the rules' own tick, a pick, and the invariant every 60th tick.
fn scripted_run() -> Engine {
    let mut e = sword();
    for tick in 1..=3000u32 {
        if tick % 250 == 0 {
            let (_, state) = world_state(&mut e);
            let g = state
                .players
                .career_mut(PlayerIdx::SOLO)
                .expertise
                .as_mut()
                .expect("a gauge");
            g.points = g.points.max(g.next);
        }
        if tick == 1600 {
            tree_pick(&mut e, 1).expect("tempo");
        }
        e.step(&[Input::default(); crate::id::MAX_PLAYERS as usize]);
        if tick % 60 == 0 {
            assert_set_invariant(&e);
        }
    }
    e
}

fn assert_set_invariant(e: &Engine) {
    let (style, level) = (gauge(e).style, gauge(e).level);
    let (world, _) = match e.run() {
        crate::engine::Run::V2 { world, state } => (world, state),
        crate::engine::Run::V1 { .. } => panic!("the live rules"),
    };
    let want = expertise::set_for(world, Expertise::bundled(), style, level);
    for f in StatEffect::ALL {
        if f != StatEffect::MaxHp {
            assert_eq!(tenths(e)[f as usize], want[f as usize], "{f:?}");
        }
    }
}

/// S8: the arrays are the set of the level and the tree, at every 60th tick of a run.
#[test]
fn the_set_arrays_are_the_set_of_the_level_and_the_tree() {
    let e = scripted_run();
    assert!(gauge(&e).level >= 3, "the script crosses levels");
    assert_eq!(gauge(&e).style, Some(1));
}

/// S9: the state continues the same from its saved form (the savepoint carries `set_tenths`):
/// a decoded copy of the career goes on level by level to the same hash as the straight run.
#[test]
fn a_resumed_run_matches_the_straight_run() {
    let mut straight = sword();
    level_to(&mut straight, 4);
    tree_pick(&mut straight, 1).expect("tempo");
    let mut resumed = sword();
    level_to(&mut resumed, 4);
    tree_pick(&mut resumed, 1).expect("tempo");
    {
        use crate::savepoint::testing::{from_bytes, to_bytes};
        let (_, state) = world_state(&mut resumed);
        let career = state.players.career_mut(PlayerIdx::SOLO);
        let back = from_bytes::<Option<super::state::Progress>>(&to_bytes(&career.progress))
            .expect("decodes");
        career.progress = back.map(|mut p| {
            p.stats = progress(&straight).stats.clone();
            p
        });
    }
    assert_eq!(resumed.state_hash(), straight.state_hash());
    for level in 5..=9 {
        level_to(&mut straight, level);
        level_to(&mut resumed, level);
        assert_eq!(resumed.state_hash(), straight.state_hash(), "level {level}");
        assert_set_invariant(&resumed);
    }
}

/// S10: levels crossed in one tick equal the same levels crossed over two ticks.
#[test]
fn levels_crossed_in_one_tick_equal_two_ticks() {
    let (mut one, mut two) = (sword(), sword());
    level_to(&mut one, 9);
    level_to(&mut two, 3);
    level_to(&mut two, 9);
    assert_eq!(one.state_hash(), two.state_hash());
    assert_eq!(tenths(&one), tenths(&two));
}

/// S11: the placeholder tables are pinned: the tempo rows written out, the others by
/// `floor(M * n / 10)`, ten values each, only the allowed effects, each within its bound.
#[test]
fn the_set_values_are_pinned() {
    let tempo = expertise::TREE_SETS
        .iter()
        .find(|(w, s, _)| (*w, *s) == ("giant_sword", "tempo"))
        .expect("tempo")
        .2;
    let row = |e: StatEffect| tempo.iter().find(|(f, _)| *f == e).expect("a row").1;
    assert_eq!(
        row(StatEffect::AttackPct),
        [33, 66, 99, 132, 165, 198, 231, 264, 297, 330]
    );
    assert_eq!(
        row(StatEffect::AttackSpeedPct),
        [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000]
    );
    assert_eq!(
        row(StatEffect::SkillCooldownPct),
        [20, 40, 60, 80, 100, 120, 140, 160, 180, 200]
    );
    assert_eq!(
        row(StatEffect::RangePct),
        [50, 100, 150, 200, 250, 300, 350, 400, 450, 500]
    );

    let lv10 = |w: &str, s: &str| -> Vec<(StatEffect, u32)> {
        expertise::TREE_SETS
            .iter()
            .find(|(a, b, _)| (*a, *b) == (w, s))
            .expect("a tree")
            .2
            .iter()
            .map(|(f, v)| (*f, v[9]))
            .collect()
    };
    use StatEffect::{AttackPct, AttackSpeedPct, EvadePct, MaxHp, RangePct, SkillCooldownPct};
    assert_eq!(
        lv10("giant_sword", "edge"),
        [
            (AttackPct, 500),
            (AttackSpeedPct, 300),
            (SkillCooldownPct, 100),
            (RangePct, 200)
        ]
    );
    assert_eq!(
        lv10("giant_sword", "reach"),
        [
            (AttackPct, 300),
            (AttackSpeedPct, 300),
            (SkillCooldownPct, 100),
            (RangePct, 800)
        ]
    );
    assert_eq!(
        lv10("giant_hammer", "crush"),
        [
            (AttackPct, 600),
            (AttackSpeedPct, 200),
            (SkillCooldownPct, 100),
            (RangePct, 200)
        ]
    );
    assert_eq!(
        lv10("giant_hammer", "quake"),
        [
            (AttackPct, 300),
            (AttackSpeedPct, 200),
            (SkillCooldownPct, 200),
            (RangePct, 800)
        ]
    );
    assert_eq!(
        lv10("giant_hammer", "bulwark"),
        [
            (AttackPct, 200),
            (SkillCooldownPct, 200),
            (MaxHp, 40000),
            (EvadePct, 200)
        ]
    );
    for w in ["giant_sword", "giant_hammer"] {
        let base = expertise::BASE_SETS
            .iter()
            .find(|(a, _)| *a == w)
            .expect("a base")
            .1;
        assert_eq!(base.len(), 2);
        assert_eq!(
            base[0],
            (AttackPct, [10, 20, 30, 40, 50, 60, 70, 80, 90, 100])
        );
        assert_eq!(
            base[1],
            (AttackSpeedPct, [10, 20, 30, 40, 50, 60, 70, 80, 90, 100])
        );
    }
    let sets = expertise::TREE_SETS
        .iter()
        .map(|(_, _, s)| *s)
        .chain(expertise::BASE_SETS.iter().map(|(_, s)| *s));
    for set in sets {
        for (f, values) in set {
            assert!(
                [
                    AttackPct,
                    AttackSpeedPct,
                    EvadePct,
                    SkillCooldownPct,
                    RangePct,
                    MaxHp
                ]
                .contains(f),
                "{f:?}"
            );
            for (i, v) in values.iter().enumerate() {
                assert_eq!(
                    *v,
                    values[9] * u32::try_from(i + 1).unwrap() / 10,
                    "{f:?} Lv{}",
                    i + 1
                );
                let bound = if *f == MaxHp {
                    crate::rewards::MAX_HP_AMOUNT
                } else {
                    crate::run_stats::MAX_STEP * 10
                };
                assert!(*v <= bound, "{f:?} {v}");
            }
        }
    }
}

/// S12: a downed player still moves its set with its level.
#[test]
fn a_downed_player_keeps_its_set() {
    let mut e = sword();
    world_state(&mut e).1.players[PlayerIdx::SOLO].hp = 0;
    level_to(&mut e, 5);
    assert_eq!((tenths(&e)[AP], tenths(&e)[AS]), (50, 50));
}
