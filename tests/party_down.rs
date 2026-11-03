//! Down state, out timer, teammate revive, offer handling and party wipe (ADR-SIM-7, ADR-SIM-8).

use serde_json::json;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{OfferMode, RunSetup};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::DownView;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::version::Versions;

use crate::common;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn lethal_setup(damage: u32, interval: u32) -> (RunSetup, RunContent) {
    let mut v: serde_json::Value =
        serde_json::from_slice(common::bundled_file("run_stats")).unwrap();
    v["enemy_attack_damage"] = json!(damage);
    let stats = RunStats::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();

    let mut tlv: serde_json::Value =
        serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    tlv["waves"] = json!([{
        "at_tick": 1,
        "enemy": {
            "base": "bat", "hp": 9_000_000, "radius": 2048,
            "movement": { "type": "straight", "vx": 0, "vy": 0 },
            "rules": [{
                "when": { "type": "always" },
                "target": "nearest_ally",
                "do": {
                    "type": "attack",
                    "attack": {
                        "preset": { "id": "aimed_single", "args": [interval] }
                    }
                }
            }]
        },
        "spawn": { "x": 0, "y": 0 }
    }]);
    let mode = TimeLimit::parse(tlv.to_string().as_bytes(), Versions::CURRENT).unwrap();
    let content = RunContent::bundled()
        .clone()
        .with_run_stats(stats)
        .with_time_limit(mode);
    let setup = RunSetup {
        mode: stella_rain_core::id::ContentId::from("time_limit"),
        seed: 42,
        ..Default::default()
    };
    (setup, content)
}

/// A. Down state and out timer (ADR-SIM-7, ADR-SIM-8).
#[test]
fn down_state_and_out_timer() {
    let (setup, content) = lethal_setup(50_000, 15);
    let mut e = Engine::start_party_with(&setup, &content, 2).unwrap();

    let mut p0_dash = Input {
        move_x: 100,
        evade: true,
        ..Input::default()
    };
    let idle = Input::default();
    while e.snapshot().players[1].hp > 0 && e.tick() < 60 {
        e.step(&[p0_dash, idle, idle]);
        p0_dash.evade = false;
        p0_dash.move_x = 0;
    }

    let snap = e.snapshot();
    assert_eq!(snap.players[1].hp, 0);
    assert!(snap.players[1].down.is_some());
    assert!(!snap.players[1].out);
    let down = snap.players[1].down.unwrap();
    assert_eq!(down.ticks_left, 1200);
    assert_eq!(down.revive_hold, 0);
    assert!(snap.players[0].hp > 0);
    assert!(snap.players[0].down.is_none());
    assert!(!snap.players[0].out);

    // Player 1 cannot move or attack while downed.
    let at_before = snap.players[1].at;
    let p1_act = Input {
        move_x: 100,
        main: true,
        ..Input::default()
    };
    e.step(&[idle, p1_act, idle]);
    let snap2 = e.snapshot();
    assert_eq!(snap2.players[1].at, at_before);
    assert!(snap2.players[1].combo.presses.is_empty());

    // Advance 1,200 ticks without teammate interact: player 1 transitions to out.
    for _ in 0..1199 {
        e.step(&[idle, idle, idle]);
    }
    let snap_out = e.snapshot();
    assert!(snap_out.players[1].out);
    assert!(snap_out.players[1].down.is_none());

    // Out player cannot be revived by teammate holding interact in range.
    let p0_interact = Input {
        interact: true,
        ..Input::default()
    };
    e.step(&[p0_interact, idle, idle]);
    let snap_after = e.snapshot();
    assert!(snap_after.players[1].out);
    assert!(snap_after.players[1].down.is_none());
}

/// B. Teammate revive with interact button (ADR-SIM-7, ADR-SIM-8).
#[test]
fn teammate_revive_with_interact_button() {
    let (setup, content) = lethal_setup(50_000, 15);
    let mut e = Engine::start_party_with(&setup, &content, 2).unwrap();

    let mut p0_dash = Input {
        move_x: 50,
        evade: true,
        ..Input::default()
    };
    let idle = Input::default();
    while e.snapshot().players[1].hp > 0 && e.tick() < 60 {
        e.step(&[p0_dash, idle, idle]);
        p0_dash.evade = false;
        p0_dash.move_x = 0;
    }
    assert!(e.snapshot().players[1].down.is_some());

    // Teammate holding interact in range (<= 48 px / 12,288 units) increments revive_hold.
    let p0_interact = Input {
        interact: true,
        ..Input::default()
    };
    e.step(&[p0_interact, idle, idle]);
    let snap = e.snapshot();
    assert_eq!(snap.players[1].down.unwrap().revive_hold, 1);

    // Releasing interact resets revive_hold to 0.
    e.step(&[idle, idle, idle]);
    assert_eq!(e.snapshot().players[1].down.unwrap().revive_hold, 0);

    // Continuous hold for 180 ticks revives player 1 at 30% max HP with invulnerability.
    for _ in 0..180 {
        e.step(&[p0_interact, idle, idle]);
    }
    let revived_snap = e.snapshot();
    assert_eq!(revived_snap.players[1].hp, 15_000); // 30% of 50,000
    assert_eq!(revived_snap.players[1].invulnerable, 60);
    assert!(revived_snap.players[1].down.is_none());
    assert!(!revived_snap.players[1].out);
}

/// C. Open timed offer auto-picks on down (ADR-SIM-7, ADR-SIM-8).
#[test]
fn open_timed_offer_auto_picks_on_down() {
    let (mut setup, content) = lethal_setup(50_000, 15);
    setup.offer_mode = OfferMode::Timed;
    let e = Engine::start_party_with(&setup, &content, 2).unwrap();
    assert_eq!(e.present(), 0b011);
}

/// D. Wipe in decide_outcome (ADR-SIM-7, ADR-SIM-8).
#[test]
fn wipe_in_decide_outcome() {
    let (setup, content) = lethal_setup(50_000, 15);

    // Solo: 0 HP fails immediately.
    let mut e_solo = Engine::start_party_with(&setup, &content, 1).unwrap();
    let idle = Input::default();
    while e_solo.outcome() == Outcome::Running && e_solo.tick() < 60 {
        e_solo.step(&[idle, idle, idle]);
    }
    assert_eq!(e_solo.outcome(), Outcome::Failed);

    // Party: 1 downed, 1 up -> stays Running.
    let mut e_party = Engine::start_party_with(&setup, &content, 2).unwrap();
    let mut p0_dash = Input {
        move_x: 100,
        evade: true,
        ..Input::default()
    };
    while e_party.snapshot().players[1].hp > 0 && e_party.tick() < 60 {
        e_party.step(&[p0_dash, idle, idle]);
        p0_dash.evade = false;
        p0_dash.move_x = 0;
    }
    assert_eq!(e_party.snapshot().players[1].hp, 0);
    assert_eq!(e_party.outcome(), Outcome::Running);

    // When all players are down/out (!any_up()), the run fails on that tick.
    while e_party.outcome() == Outcome::Running && e_party.tick() < 120 {
        e_party.step(&[Input { move_x: -100, ..Input::default() }, idle, idle]);
    }
    assert_eq!(e_party.outcome(), Outcome::Failed);
}

/// E. Domain events and DownView round-trip (ADR-SIM-7, ADR-SIM-8).
#[test]
fn down_and_revive_events_and_snapshot_views() {
    let downed = DomainEvent::PlayerDowned { player: slot(1) };
    let revived = DomainEvent::PlayerRevived {
        player: slot(1),
        reviver: slot(0),
    };
    let out = DomainEvent::PlayerOut { player: slot(1) };

    for ev in [&downed, &revived, &out] {
        let json = serde_json::to_string(ev).unwrap();
        let back: DomainEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(*ev, back);
    }

    let down_view = DownView {
        ticks_left: 1200,
        revive_hold: 60,
    };
    let json = serde_json::to_string(&down_view).unwrap();
    let back: DownView = serde_json::from_str(&json).unwrap();
    assert_eq!(down_view, back);
}
