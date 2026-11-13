//! The run rules' rally (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7, ADR-SIM-8): a hit a player
//! takes leaves part of its hit points as rally for a while, the player's own combo hits win it
//! back, and the rest is lost when the ticks run out or the player is downed. Every number is a
//! test value, not content.

use super::trigger_tests::{
    ME, Rig, count, full_effects, named, rig, slot, st, trigger_kind, world,
};
use crate::input::Input;
use crate::rules::Rally;
use crate::run_stats::StatEffect;
use serde_json::{Value, json};

/// A rig of `players` whose rules name a rally of `pct` percent for `ticks` ticks that wins back
/// `regain` percent, on a style with the trigger (`effects`) when it has any.
fn tuned(players: u8, pct: u8, ticks: u16, regain: u32, effects: Vec<Value>, foes: u32) -> Rig {
    let kinds = if effects.is_empty() {
        vec![]
    } else {
        vec![trigger_kind(effects)]
    };
    let mut r = rig(players, kinds, foes);
    world(&mut r.e).tuning.rally = Some(Rally {
        pct,
        ticks,
        regain_pct: regain,
    });
    r
}

fn combat(r: &mut Rig, i: u8) -> (u32, u16) {
    let c = st(&mut r.e).players[slot(i)].combat;
    (c.rally, c.rally_ticks)
}

fn hp(r: &mut Rig, i: u8) -> u32 {
    st(&mut r.e).players[slot(i)].hp
}

/// B10: a hit leaves rally, a second adds to it, it never passes the hit points missing, a hit that
/// took nothing leaves none.
#[test]
fn a_hit_leaves_rally() {
    let mut r = tuned(1, 50, 180, 100, vec![], 0);
    let ev = r.hurt(0, 200, false);
    assert_eq!(
        named(&ev, "RallyGained"),
        [json!({ "player": "first", "amount": 100, "rally": 100 })]
    );
    assert_eq!(combat(&mut r, 0), (100, 180));
    r.idle(10);
    r.hurt(0, 100, false);
    assert_eq!(combat(&mut r, 0), (150, 180), "added, ticks restarted");

    let ev = r.hurt(0, 0, false);
    assert_eq!(count(&ev, "RallyGained"), 0, "a hit that took nothing");
    assert_eq!(combat(&mut r, 0), (150, 180));

    let mut full = tuned(1, 100, 180, 100, vec![], 0);
    full.hurt(0, 900, false);
    assert_eq!(combat(&mut full, 0).0, 900);
    full.hurt(0, 50, false);
    assert_eq!(combat(&mut full, 0).0, 950, "min(900 + 50, 1000 - 50)");

    let mut big = tuned(1, 100, 180, 100, vec![], 0);
    {
        let p = &mut st(&mut big.e).players[ME];
        p.hp = 2_000;
        p.max_hp = 2_000;
    }
    big.hurt(0, 1_500, false);
    assert_eq!(combat(&mut big, 0).0, 1_500, "at most the 1500 missing");
}

/// B10: a downed player gains none, and none is left alone when the rules name no rally.
#[test]
fn a_downed_player_and_a_run_without_rules_gain_none() {
    let mut r = tuned(2, 50, 180, 100, vec![], 0);
    r.hurt(0, 1_000_000, false);
    assert_eq!(
        combat(&mut r, 0),
        (0, 0),
        "downed in one hit: nothing to win back"
    );
    r.hurt(0, 50, false);
    assert_eq!(combat(&mut r, 0), (0, 0), "a downed player");
    let mut off = rig(1, vec![], 0);
    off.hurt(0, 200, false);
    assert_eq!(count(&off.events, "RallyGained"), 0);
    assert_eq!(st(&mut off.e).players[ME].combat.rally, 0);
}

/// B9: the trigger's `rally_pct` scales what a hit adds while it runs.
#[test]
fn the_trigger_scales_the_rally() {
    let doubled = vec![json!({ "type": "rally_pct", "pct": 200 })];
    let mut r = tuned(1, 50, 180, 100, doubled.clone(), 0);
    r.hurt(0, 100, false);
    assert_eq!(combat(&mut r, 0).0, 50, "off");
    let mut r = tuned(1, 50, 180, 100, doubled, 0);
    r.set_trigger(0, 100);
    r.hurt(0, 100, false);
    assert_eq!(combat(&mut r, 0).0, 100, "on");
    let mut r = tuned(1, 50, 180, 100, full_effects(), 0);
    r.set_trigger(0, 100);
    r.hurt(0, 100, false);
    assert_eq!(combat(&mut r, 0).0, 50, "no rally_pct effect");
}

/// B11: the player's own combo hit wins back `regain` percent of the damage it dealt, from the rally,
/// as hit points.
#[test]
fn a_combo_hit_wins_the_rally_back() {
    let win = |regain: u32, rally: u32| {
        let mut r = tuned(1, 50, 180, regain, vec![], 1);
        {
            let c = &mut st(&mut r.e).players[ME];
            c.hp = 500;
            c.combat.rally = rally;
            c.combat.rally_ticks = 180;
        }
        let ev = r.run_step(false);
        let (left, ticks) = combat(&mut r, 0);
        let events = named(&ev, "RallyWon");
        let won = events
            .iter()
            .map(|e| u32::try_from(e["amount"].as_u64().unwrap_or(0)).unwrap_or(0))
            .sum();
        assert!(
            hp(&mut r, 0) >= 500 + won,
            "the hit points rose by what was won"
        );
        (won, left, ticks, events)
    };
    let (won, left, _, events) = win(100, 400);
    assert!(won > 0, "a hit won hit points back");
    assert_eq!(left, 400 - won);
    assert_eq!(events.len(), 1, "one hit, one event");
    assert_eq!(
        events[0],
        json!({ "player": "first", "amount": won, "rally": 400 - won })
    );
    let (half, ..) = win(50, 400);
    assert_eq!(half, won / 2, "half the share");
    // The rally is the limit, and it ends with it.
    let (small, left, ticks, _) = win(100, 1);
    assert_eq!((small, left, ticks), (1, 0, 0));
    // Nothing missing: nothing won, the rally stays.
    let mut r = tuned(1, 50, 180, 100, vec![], 1);
    {
        let c = &mut st(&mut r.e).players[ME];
        c.combat.rally = 100;
        c.combat.rally_ticks = 180;
    }
    r.run_step(false);
    assert_eq!(hp(&mut r, 0), 1_000);
    assert_eq!(combat(&mut r, 0).0, 100);
    assert_eq!(count(&r.events, "RallyWon"), 0);
}

/// B12: the rest is lost when the ticks run out, a hit restarts them, a down loses it at once.
#[test]
fn the_rally_lapses() {
    let mut r = tuned(2, 50, 180, 100, vec![], 0);
    r.hurt(0, 200, false);
    r.idle(179);
    assert_eq!(combat(&mut r, 0), (100, 1));
    let ev = r.step([Input::default(); 3]);
    assert_eq!(named(&ev, "RallyLapsed"), [json!({ "player": "first" })]);
    assert_eq!(combat(&mut r, 0), (0, 0));
    r.idle(5);
    assert_eq!(count(&r.events, "RallyLapsed"), 1, "once");

    r.hurt(0, 200, false);
    r.idle(100);
    r.hurt(0, 100, false);
    r.idle(100);
    assert!(combat(&mut r, 0).0 > 0, "a hit restarted the count");

    let ev = r.hurt(0, 1_000_000, false);
    assert_eq!(count(&ev, "PlayerDowned"), 1);
    assert_eq!(count(&ev, "RallyLapsed"), 1);
    assert_eq!(combat(&mut r, 0), (0, 0));
}

/// B13: the view carries the rally while it is above 0 and no key otherwise.
#[test]
fn the_view_follows_the_rally() {
    let mut r = tuned(1, 50, 180, 100, vec![], 0);
    assert!(r.view(0).get("rally").is_none());
    r.hurt(0, 200, false);
    assert_eq!(r.view(0)["rally"], json!({ "hp": 100, "ticks_left": 180 }));
    r.idle(1);
    assert_eq!(r.view(0)["rally"], json!({ "hp": 100, "ticks_left": 179 }));
    let mut off = rig(1, vec![], 0);
    off.hurt(0, 200, false);
    assert!(off.view(0).get("rally").is_none());
}

/// B14: where the most hit points fall, the rally is clamped to them so the codec's invariant holds.
#[test]
fn a_lower_max_hp_clamps_the_rally() {
    let mut r = tuned(1, 50, 180, 100, vec![], 0);
    {
        let (player, career) = st(&mut r.e).players.both_mut(ME);
        player.max_hp = 1_000;
        player.hp = 400;
        player.combat.rally = 500;
        player.combat.rally_ticks = 100;
        let progress = career.progress.as_mut().expect("levels");
        progress.totals[StatEffect::MaxHp as usize] = 600;
        super::levels::remove_effect(progress, player, StatEffect::MaxHp, 600);
        assert_eq!(player.max_hp, 400);
        assert_eq!(player.combat.rally, 400, "clamped to the new most");
    }
    {
        let (player, career) = st(&mut r.e).players.both_mut(ME);
        player.combat.rally = 0;
        let progress = career.progress.as_mut().expect("levels");
        progress.totals[StatEffect::MaxHp as usize] = 100;
        super::levels::remove_effect(progress, player, StatEffect::MaxHp, 100);
        assert_eq!(player.combat.rally, 0, "no rally: untouched");
    }
}

/// B17: in a party a hit and a win back belong to one player.
#[test]
fn each_player_has_their_own_rally() {
    let mut r = tuned(3, 50, 180, 100, vec![], 1);
    r.hurt(1, 200, false);
    assert_eq!(combat(&mut r, 0).0, 0);
    assert_eq!(combat(&mut r, 1).0, 100);
    assert_eq!(combat(&mut r, 2).0, 0);
    r.hurt(2, 1_000_000, false);
    assert_eq!(
        combat(&mut r, 1).0,
        100,
        "player 2's down loses only its own"
    );
}

/// B18: a savepoint taken mid-state gives the same hashes as the unbroken run.
#[test]
fn a_savepoint_mid_state_replays_the_same() {
    let go = |r: &mut Rig| {
        r.hurt(0, 200, false);
        r.set_trigger(0, 200);
        r.idle(10);
    };
    let mut a = tuned(1, 50, 180, 100, full_effects(), 0);
    go(&mut a);
    let bytes = crate::savepoint::encode_state(&a.e);
    let mut b = tuned(1, 50, 180, 100, full_effects(), 0);
    go(&mut b);
    assert_eq!(a.e.state_hash(), b.e.state_hash());
    assert!(!bytes.is_empty());
    for _ in 0..240 {
        a.step([Input::default(); 3]);
        b.step([Input::default(); 3]);
        assert_eq!(a.e.state_hash(), b.e.state_hash());
    }
}
