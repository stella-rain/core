//! A style path's spirit, counter bonus, combo tempo and giant trigger stack across its levels
//! as the breaker does (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SVC-1): the style's own level,
//! then each level taken, in path order; a later level's number replaces the earlier one and an
//! effect it does not list stays. Every number is a test value, not content.

use super::state::PlayerIdx;
use super::trigger_tests::{levelled_pack, slot, st};
use crate::engine::{Engine, Run};
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use serde_json::{Value, json};

/// A three-slot party run on the Hammer whose style 0 names a gauge, a bleed smash, then `style` kinds,
/// its first upgrade `up_1`, its second `up_2` and its capstone `cap`.
fn stacked(style: Vec<Value>, up_1: Vec<Value>, up_2: Vec<Value>, cap: Vec<Value>) -> Engine {
    // The bleed smash a spirit's bleed effect needs; the pack check reads it off the style's tree.
    let mut kinds = vec![json!({ "type": "smash_effect", "effect": { "type": "bleed" } })];
    kinds.extend(style);
    let pack: Pack = levelled_pack(kinds, up_1, up_2, cap);
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_hammer".into(),
        seed: 20_261_117,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party_with(&setup, pack.content(), 3).expect("a run");
    for i in 0..3 {
        if let Some(g) = st(&mut e).players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(0);
        }
    }
    e
}

/// Player `i` has taken `places` places of its path.
fn take(e: &mut Engine, i: u8, places: usize) {
    let path = &mut st(e)
        .players
        .career_mut(slot(i))
        .expertise
        .as_mut()
        .expect("a gauge")
        .path;
    path.clear();
    path.extend(std::iter::repeat_n(0, places));
}

/// What player `i` reads now, through the world's accessor `read`, at `places` places taken.
fn read<T>(
    e: &mut Engine,
    i: u8,
    places: usize,
    read: impl Fn(&super::world::World, &super::state::State, PlayerIdx) -> Option<T>,
) -> Option<T> {
    take(e, i, places);
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    read(world, state, slot(i))
}

fn spirit(decay: u32, levels: Value) -> Value {
    json!({ "type": "spirit", "decay_ticks": decay, "levels": levels })
}

fn dmg(pct: u32) -> Value {
    json!({ "type": "damage_pct", "pct": pct })
}

fn range(pct: u32) -> Value {
    json!({ "type": "range_pct", "pct": pct })
}

fn bleed(stacks: u32) -> Value {
    json!({ "type": "smash_bleed_stacks", "stacks": stacks })
}

/// A spirit as `(decay_ticks, [(damage, range, bleed)])`.
type SpiritSeen = (u16, Vec<(u32, u32, u32)>);

fn spirit_at(e: &mut Engine, i: u8, places: usize) -> Option<SpiritSeen> {
    read(e, i, places, |w, s, me| w.spirit_of(s, me).cloned()).map(|g| {
        (
            g.decay_ticks,
            g.levels
                .iter()
                .map(|f| (f.damage_pct, f.range_pct, f.bleed_stacks))
                .collect(),
        )
    })
}

/// G1: the spirit's countdown and level effects stack in path order, a later level replacing the
/// number it names, an empty entry changing nothing and a longer table extending the earlier one.
#[test]
fn the_spirit_stacks_across_the_path() {
    let base = spirit(
        1200,
        json!([[dmg(8)], [dmg(16), range(15)], [dmg(25), bleed(2)]]),
    );
    let mut e = stacked(
        vec![base],
        vec![spirit(1800, json!([[]]))],
        vec![],
        vec![spirit(1800, json!([[dmg(10)]]))],
    );
    let own = vec![(8, 0, 0), (16, 15, 0), (25, 0, 2)];
    assert_eq!(spirit_at(&mut e, 0, 0), Some((1200, own.clone())));
    assert_eq!(spirit_at(&mut e, 0, 1), Some((1800, own)));
    let last = spirit_at(&mut e, 0, 3).expect("a spirit");
    assert_eq!(last.0, 1800);
    assert_eq!(last.1, vec![(10, 0, 0), (16, 15, 0), (25, 0, 2)]);

    // A longer later table extends the earlier one (to three levels).
    let mut e = stacked(
        vec![spirit(600, json!([[dmg(8)], [dmg(16)]]))],
        vec![spirit(600, json!([[], [], [dmg(30)]]))],
        vec![],
        vec![],
    );
    assert_eq!(
        spirit_at(&mut e, 0, 1).map(|g| g.1),
        Some(vec![(8, 0, 0), (16, 0, 0), (30, 0, 0)])
    );
}

/// G1: each player reads its own style and path; a style naming none gives none.
#[test]
fn each_player_reads_its_own_spirit() {
    let mut e = stacked(
        vec![spirit(1200, json!([[dmg(8)]]))],
        vec![spirit(1800, json!([[]]))],
        vec![],
        vec![],
    );
    take(&mut e, 2, 1);
    assert_eq!(spirit_at(&mut e, 0, 0).map(|g| g.0), Some(1200));
    assert_eq!(spirit_at(&mut e, 2, 1).map(|g| g.0), Some(1800));
    if let Some(g) = st(&mut e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(1);
    }
    assert_eq!(spirit_at(&mut e, 1, 0), None, "a style naming none");
}

/// G1: a later kind past every cap (a value the validator refuses) is clamped as a single
/// level's is.
#[test]
fn a_later_spirit_past_the_caps_is_clamped() {
    use super::world::{SpiritGrant, spirit_fold};
    let kind = |v: Value| serde_json::from_value::<crate::expertise::Spirit>(v).expect("a spirit");
    let mut g = SpiritGrant::none();
    spirit_fold(&mut g, &kind(json!({ "decay_ticks": 100, "levels": [[]] })));
    spirit_fold(
        &mut g,
        &kind(json!({
            "decay_ticks": 4_000_000_000u32,
            "levels": [[dmg(u32::MAX), range(u32::MAX), bleed(u32::MAX)], [], [], [dmg(5)]]
        })),
    );
    assert_eq!(g.decay_ticks, crate::limits::MAX_SPIRIT_DECAY_TICKS);
    assert_eq!(g.levels.len(), usize::from(crate::limits::MAX_SPIRIT_LEVEL));
    let fx = g.levels[0];
    assert_eq!(fx.damage_pct, crate::limits::MAX_PCT);
    assert_eq!(fx.range_pct, crate::rewards::MAX_RANGE_PCT);
    assert_eq!(fx.bleed_stacks, crate::expertise::MAX_SPIRIT_BLEED_STACKS);
}

fn counter(window: u32, effects: Vec<Value>) -> Value {
    json!({ "type": "counter_bonus", "window_ticks": window, "effects": effects })
}

/// A counter grant as `(window, damage_pct, stagger_pct, stun_ticks, advance)`.
type CounterSeen = (u16, u32, u32, u32, i32);

fn counter_at(e: &mut Engine, i: u8, places: usize) -> Option<CounterSeen> {
    read(e, i, places, |w, s, me| w.counter_of(s, me).cloned()).map(|g| {
        (
            g.window,
            g.damage_pct,
            g.stagger_pct,
            g.stun_ticks,
            g.advance.0,
        )
    })
}

/// G2: the counter's window and each effect stack in path order; an effect a later level does not
/// list stays.
#[test]
fn the_counter_bonus_stacks_across_the_path() {
    let base = counter(
        30,
        vec![
            json!({ "type": "damage_pct", "pct": 100 }),
            json!({ "type": "stagger_pct", "pct": 300 }),
            json!({ "type": "stun_ticks", "ticks": 45 }),
        ],
    );
    let mut e = stacked(
        vec![base],
        vec![counter(45, vec![])],
        vec![counter(
            45,
            vec![json!({ "type": "advance", "distance": 12288 })],
        )],
        vec![counter(
            20,
            vec![json!({ "type": "damage_pct", "pct": 150 })],
        )],
    );
    assert_eq!(counter_at(&mut e, 0, 0), Some((30, 100, 300, 45, 0)));
    assert_eq!(counter_at(&mut e, 0, 1), Some((45, 100, 300, 45, 0)));
    assert_eq!(counter_at(&mut e, 0, 2), Some((45, 100, 300, 45, 12288)));
    assert_eq!(counter_at(&mut e, 0, 3), Some((20, 150, 300, 45, 12288)));
}

/// G2: each player reads its own place on the path; a style naming none gives none.
#[test]
fn each_player_reads_its_own_counter() {
    let mut e = stacked(
        vec![counter(
            30,
            vec![json!({ "type": "damage_pct", "pct": 100 })],
        )],
        vec![counter(45, vec![])],
        vec![],
        vec![],
    );
    assert_eq!(counter_at(&mut e, 0, 0).map(|g| g.0), Some(30));
    assert_eq!(counter_at(&mut e, 2, 1).map(|g| g.0), Some(45));
    assert_eq!(counter_at(&mut e, 0, 0).map(|g| g.0), Some(30));
    if let Some(g) = st(&mut e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(1);
    }
    assert_eq!(counter_at(&mut e, 1, 1), None, "a style naming none");
}
