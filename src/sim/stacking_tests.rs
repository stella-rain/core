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
/// G2: a later counter past every cap (a value the validator refuses) is clamped as a single
/// level's is.
#[test]
fn a_later_counter_past_the_caps_is_clamped() {
    use super::world::{CounterGrant, counter_fold};
    let kind = |mut v: Value| {
        v.as_object_mut().expect("a kind").remove("type");
        serde_json::from_value::<crate::expertise::CounterBonus>(v).expect("a counter bonus")
    };
    let mut g = CounterGrant::none();
    counter_fold(&mut g, &kind(counter(30, vec![])));
    counter_fold(
        &mut g,
        &kind(counter(
            4_000_000_000,
            vec![
                json!({ "type": "damage_pct", "pct": u32::MAX }),
                json!({ "type": "stagger_pct", "pct": u32::MAX }),
                json!({ "type": "stun_ticks", "ticks": u32::MAX }),
                json!({ "type": "advance", "distance": i32::MAX }),
            ],
        )),
    );
    assert_eq!(g.window, crate::limits::MAX_COUNTER_WINDOW_TICKS);
    assert_eq!(g.damage_pct, crate::limits::MAX_PCT);
    assert_eq!(g.stagger_pct, crate::limits::MAX_PCT);
    assert_eq!(g.stun_ticks, crate::validate::limits::MAX_STUN_TICKS);
    assert_eq!(
        g.advance,
        crate::fixed::Fx::px(crate::limits::MAX_ADVANCE_DISTANCE)
    );
}

fn tempo(max: u32, effects: Vec<Value>) -> Value {
    json!({ "type": "combo_tempo_pct", "max_stacks": max, "effects": effects })
}

fn speed(pct: u32) -> Value {
    json!({ "type": "speed_pct", "pct": pct })
}

/// A tempo grant as `(max_stacks, speed_pct, keep_on_smash, hold_armour)`.
type TempoSeen = (u8, u32, bool, bool);

fn tempo_at(e: &mut Engine, i: u8, places: usize) -> Option<TempoSeen> {
    read(e, i, places, |w, s, me| w.tempo_of(s, me).copied())
        .map(|g| (g.max_stacks, g.speed_pct, g.keep_on_smash, g.hold_armour))
}

/// G3: the tempo's cap and each effect stack in path order; the flag effects are true once any
/// level names them.
#[test]
fn the_combo_tempo_stacks_across_the_path() {
    let keep = json!({ "type": "keep_on_smash" });
    let armour = json!({ "type": "super_armour_at_full" });
    let mut e = stacked(
        vec![tempo(5, vec![speed(4), keep])],
        vec![tempo(6, vec![])],
        vec![tempo(6, vec![speed(5), armour])],
        vec![],
    );
    assert_eq!(tempo_at(&mut e, 0, 0), Some((5, 4, true, false)));
    assert_eq!(tempo_at(&mut e, 0, 1), Some((6, 4, true, false)));
    assert_eq!(tempo_at(&mut e, 0, 2), Some((6, 5, true, true)));
    assert_eq!(tempo_at(&mut e, 0, 3), Some((6, 5, true, true)));
}

/// G3: each player reads its own place on the path; a style naming none gives none.
#[test]
fn each_player_reads_its_own_tempo() {
    let mut e = stacked(
        vec![tempo(5, vec![speed(4)])],
        vec![tempo(6, vec![])],
        vec![],
        vec![],
    );
    assert_eq!(tempo_at(&mut e, 0, 0).map(|g| g.0), Some(5));
    assert_eq!(tempo_at(&mut e, 2, 1).map(|g| g.0), Some(6));
    if let Some(g) = st(&mut e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(2);
    }
    assert_eq!(tempo_at(&mut e, 1, 1), None, "a style naming none");
}

/// G3: a later tempo past every cap (a value the validator refuses) is clamped as a single
/// level's is.
#[test]
fn a_later_tempo_past_the_caps_is_clamped() {
    use super::world::{TempoGrant, tempo_fold};
    let kind = |mut v: Value| {
        v.as_object_mut().expect("a kind").remove("type");
        serde_json::from_value::<crate::expertise::ComboTempo>(v).expect("a combo tempo")
    };
    let mut g = TempoGrant::none();
    tempo_fold(&mut g, &kind(tempo(5, vec![speed(4)])));
    tempo_fold(&mut g, &kind(tempo(4_000_000_000, vec![speed(u32::MAX)])));
    assert_eq!(g.max_stacks, crate::limits::MAX_TEMPO_STACKS);
    assert_eq!(g.speed_pct, crate::run_stats::MAX_ATTACK_SPEED_PCT);
}

fn trigger(ticks: u32, effects: Vec<Value>) -> Value {
    json!({ "type": "giant_trigger", "ticks": ticks, "effects": effects })
}

fn pct(kind: &str, pct: u32) -> Value {
    json!({ "type": kind, "pct": pct })
}

/// A trigger grant as `(ticks, range_pct, smash_pct, rally_pct, super_armour)`.
type TriggerSeen = (u16, u32, u32, u32, bool);

fn trigger_at(e: &mut Engine, i: u8, places: usize) -> Option<TriggerSeen> {
    read(e, i, places, |w, s, me| w.trigger_of(s, me).copied()).map(|g| {
        (
            g.ticks,
            g.range_pct,
            g.smash_pct,
            g.rally_pct,
            g.super_armour,
        )
    })
}

/// G4: the trigger's length and each effect stack in path order; an effect a later level does not
/// list stays.
#[test]
fn the_giant_trigger_stacks_across_the_path() {
    let base = trigger(
        480,
        vec![
            pct("range_pct", 40),
            pct("smash_pct", 20),
            json!({ "type": "super_armour" }),
        ],
    );
    let mut e = stacked(
        vec![base],
        vec![trigger(720, vec![])],
        vec![trigger(720, vec![pct("range_pct", 60)])],
        vec![trigger(720, vec![pct("rally_pct", 150)])],
    );
    assert_eq!(trigger_at(&mut e, 0, 0), Some((480, 40, 20, 100, true)));
    assert_eq!(trigger_at(&mut e, 0, 1), Some((720, 40, 20, 100, true)));
    assert_eq!(trigger_at(&mut e, 0, 2), Some((720, 60, 20, 100, true)));
    assert_eq!(trigger_at(&mut e, 0, 3), Some((720, 60, 20, 150, true)));
}

/// G4: each player reads its own place on the path; a style naming none gives none.
#[test]
fn each_player_reads_its_own_trigger() {
    let mut e = stacked(
        vec![trigger(480, vec![pct("range_pct", 40)])],
        vec![trigger(720, vec![])],
        vec![],
        vec![],
    );
    assert_eq!(trigger_at(&mut e, 0, 0).map(|g| g.0), Some(480));
    assert_eq!(trigger_at(&mut e, 2, 1).map(|g| g.0), Some(720));
    if let Some(g) = st(&mut e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(2);
    }
    assert_eq!(trigger_at(&mut e, 1, 1), None, "a style naming none");
}

/// G4: a later trigger past every cap (a value the validator refuses) is clamped as a single
/// level's is.
#[test]
fn a_later_trigger_past_the_caps_is_clamped() {
    use super::world::{TriggerGrant, trigger_fold};
    let kind = |mut v: Value| {
        v.as_object_mut().expect("a kind").remove("type");
        serde_json::from_value::<crate::expertise::GiantTrigger>(v).expect("a giant trigger")
    };
    let mut g = TriggerGrant::none();
    trigger_fold(&mut g, &kind(trigger(480, vec![])));
    trigger_fold(
        &mut g,
        &kind(trigger(
            4_000_000_000,
            vec![
                pct("range_pct", u32::MAX),
                pct("smash_pct", u32::MAX),
                pct("rally_pct", u32::MAX),
            ],
        )),
    );
    assert_eq!(g.ticks, crate::limits::MAX_TRIGGER_TICKS);
    assert_eq!(g.range_pct, crate::rewards::MAX_RANGE_PCT);
    assert_eq!(g.smash_pct, crate::limits::MAX_PCT);
    assert_eq!(g.rally_pct, crate::limits::MAX_PCT);
}

// R1 to R4 and P1: a taken upgrade changes what the rule reads.

use super::trigger_tests::{Rig, rig_levelled};

fn rig_of(style: Vec<Value>, up_1: Vec<Value>, players: u8) -> Rig {
    let mut kinds = vec![json!({ "type": "smash_effect", "effect": { "type": "bleed" } })];
    kinds.extend(style);
    rig_levelled(players, kinds, [up_1, vec![], vec![]], 3)
}

/// R1: the raised countdown is what a smash restarts, and a countdown already running is not cut.
#[test]
fn a_taken_spirit_upgrade_lengthens_the_restarted_countdown() {
    let kinds = || {
        (
            vec![spirit(1200, json!([[dmg(8)]]))],
            vec![spirit(1800, json!([[]]))],
        )
    };
    for (places, want) in [(0, 1200), (1, 1800)] {
        let (style, up) = kinds();
        let mut r = rig_of(style, up, 1);
        take(&mut r.e, 0, places);
        r.fill_gauge(0);
        r.press(true);
        assert_eq!(st(&mut r.e).players[slot(0)].combat.spirit_decay, want);
    }
    let (style, up) = kinds();
    let mut r = rig_of(style, up, 1);
    st(&mut r.e).players[slot(0)].combat.spirit_decay = 700;
    take(&mut r.e, 0, 1);
    r.idle(1);
    assert!(st(&mut r.e).players[slot(0)].combat.spirit_decay < 700);
}

/// R2: a near miss opens the raised window.
#[test]
fn a_taken_counter_upgrade_widens_the_window() {
    use crate::rules::PerfectEvade;
    let rule = PerfectEvade {
        window_ticks: 10,
        slow_ticks: 60,
        cooldown_ticks: 300,
        reach: 200 * 256,
        slow_every: 2,
    };
    for (places, want) in [(0, 30), (1, 45)] {
        let mut r = rig_of(
            vec![counter(
                30,
                vec![json!({ "type": "damage_pct", "pct": 100 })],
            )],
            vec![counter(45, vec![])],
            1,
        );
        take(&mut r.e, 0, places);
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        std::sync::Arc::make_mut(world).tuning.perfect_evade = Some(rule);
        let p = &mut state.players[slot(0)];
        p.combat.perfect_window = rule.window_ticks;
        p.combat.perfect_cooldown = 0;
        p.invulnerable = world.player.evade.invulnerable_ticks;
        super::player::near_miss(world, state, slot(0), &mut Vec::new());
        assert_eq!(state.players[slot(0)].combat.counter_window, want);
    }
}

/// R3: the raised cap lets a hit swing add a sixth stack.
#[test]
fn a_taken_tempo_upgrade_raises_the_stack_cap() {
    for (places, want) in [(0, 5), (1, 6)] {
        let mut r = rig_of(vec![tempo(5, vec![speed(4)])], vec![tempo(6, vec![])], 1);
        take(&mut r.e, 0, places);
        st(&mut r.e).players[slot(0)].combat.combo_tempo = 5;
        r.run_step(false);
        assert_eq!(st(&mut r.e).players[slot(0)].combat.combo_tempo, want);
    }
}

/// R4: a giant trigger started after the pick runs the longer length; one before it, the shorter.
#[test]
fn a_taken_trigger_upgrade_lengthens_the_state() {
    for (places, want) in [(0, 480), (1, 720)] {
        let mut r = rig_of(
            vec![trigger(480, vec![pct("range_pct", 40)])],
            vec![trigger(720, vec![])],
            1,
        );
        take(&mut r.e, 0, places);
        r.fill_gauge(0);
        r.press(true);
        let ticks = r.trigger(0);
        assert!((want - 1..=want).contains(&ticks), "{places}: {ticks}");
    }
}

/// P1: three players of one pack each read their own grants, and the run is deterministic.
#[test]
fn three_players_each_read_their_own_grants() {
    let run = |places: [usize; 3]| {
        let mut r = rig_of(
            vec![trigger(480, vec![pct("range_pct", 40)])],
            vec![trigger(720, vec![])],
            3,
        );
        for (i, n) in (0u8..).zip(places) {
            take(&mut r.e, i, n);
        }
        let seen: Vec<_> = (0u8..3)
            .map(|i| {
                let Run::V2 { world, state } = r.e.run_mut() else {
                    panic!("the live rules");
                };
                world.trigger_of(state, slot(i)).map(|g| g.ticks)
            })
            .collect();
        r.idle(600);
        (seen, r.e.state_hash())
    };
    let (seen, hash) = run([0, 1, 2]);
    assert_eq!(seen, [Some(480), Some(720), Some(720)]);
    assert_eq!(run([0, 1, 2]), (seen, hash), "the same run twice");
    assert_ne!(run([0, 0, 0]).1, hash, "the places change the run");
}
