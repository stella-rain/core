//! The sweep event (ADR-SIM-9 points 3 and 4): `SweepSet` says the swing of `player` reached the
//! rules' `sweep_min_enemies` with `count` distinct enemies. Nothing emits it yet; this pins its
//! wire form. Test code only.

use serde_json::json;
use stella_rain_core::event::DomainEvent;

/// V1: the event reads from its tagged form and writes the same form back.
#[test]
fn the_sweep_event_round_trips() {
    let wire = json!({ "type": "SweepSet", "data": { "player": "first", "count": 5 } });
    let event: DomainEvent = serde_json::from_value(wire.clone()).expect("the event parses");
    assert_eq!(serde_json::to_value(&event).unwrap(), wire);
    assert!(format!("{event:?}").contains("SweepSet"), "{event:?}");
}
