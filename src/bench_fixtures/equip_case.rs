//! The equipment rollback (ADR-SIM-6, ADR-SIM-7): `benches/rollback_equip.rs`. The dense minute of
//! `worst_minute_case`, started for three players in equipment mode; slot 0 plays the solo
//! script, slot 1 the same on the other parity, slot 2 as slot 0 (`party_case::minute_inputs`).
//! The rollback restores a saved engine, then plays 8 ticks on a clone in which every player
//! equips a star (before window tick 1) and extracts one (before window tick 5).
//!
//! Before the window the run ramps up: every 30 ticks per slot (phase `10 * slot`) the first
//! inventory star the engine accepts goes into the first slot that accepts it (the last category
//! slot stays empty for the window), never an extract, so the inventories fill. The saved engine is the run `EQUIP_PEAK_TICK` ticks in. The rollback
//! takes the saved `Engine` itself, so that the drop guard
//! (`.github/scripts/bench_drop_guard.py`) covers it. The pins are in
//! `tests/bench_fixture_pins.rs`, the allocator counts in `tests/alloc.rs`.

use super::party_case::{ROLLBACK_TICKS, minute_inputs};
use super::worst_minute_case::{self, MINUTE_START, MINUTE_TICKS};
use crate::engine::{Engine, Load};
use crate::id::PlayerIdx;
use crate::input::EquipAction;
use crate::run_setup::RunSetup;
use crate::snapshot::{SlotKindView, SlotView};

/// The ticks played before the window: the heaviest tick of the minute at which every slot's
/// inventory holds at least two items (`window` finds it; `tests/bench_fixture_pins.rs` pins it).
pub const EQUIP_PEAK_TICK: u32 = 9_834;

/// The recorded actions of the window, each with the window tick it comes before: every slot's
/// `Equip`, then every slot's `Extract`. Found for `EQUIP_PEAK_TICK` and checked by the tests.
pub const ROLLBACK_ACTIONS: [(u32, [Option<EquipAction>; 3]); 2] = [
    (
        1,
        [
            Some(EquipAction::Equip { item: 45, slot: 8 }),
            Some(EquipAction::Equip { item: 5, slot: 8 }),
            Some(EquipAction::Equip { item: 5, slot: 8 }),
        ],
    ),
    (
        5,
        [
            Some(EquipAction::Extract { item: 0 }),
            Some(EquipAction::Extract { item: 0 }),
            Some(EquipAction::Extract { item: 0 }),
        ],
    ),
];

/// The run's seed: that of the solo dense minute.
const SEED: u64 = 1;

/// A slot of the party.
fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot of the party")
}

/// The ramp-up of `s` at `tick`: the first inventory item and slot the engine accepts.
pub fn ramp(engine: &Engine, s: u8, tick: u32) -> Option<EquipAction> {
    if (tick + 10 * u32::from(s)) % 30 != 0 {
        return None;
    }
    let snap = engine.snapshot();
    let place = (engine.present() & ((1u8 << s) - 1)).count_ones() as usize;
    let view = snap.players.get(place)?;
    let kept = kept_empty(&view.slots)?;
    let slots = u32::try_from(view.slots.len()).ok()?;
    let items = u32::try_from(view.inventory.len()).ok()?;
    (0..items)
        .flat_map(|item| {
            (0..slots).map(move |slot_at| EquipAction::Equip {
                item,
                slot: slot_at,
            })
        })
        .filter(|action| !matches!(action, EquipAction::Equip { slot: at, .. } if *at == kept))
        .find(|action| engine.check_action(slot(s), *action).is_ok())
}

/// The last category slot, which the ramp-up never fills: the window's `Equip` has an empty
/// place to go to once the other slots are full.
fn kept_empty(slots: &[SlotView]) -> Option<u32> {
    let last = slots
        .iter()
        .rposition(|s| matches!(s.kind, SlotKindView::Category { .. }))?;
    u32::try_from(last).ok()
}

/// Applies the ramp-up, then steps the tick (equipment mode opens no offer).
fn play_tick(engine: &mut Engine) {
    let tick = engine.tick();
    for s in 0..3 {
        if let Some(action) = ramp(engine, s, tick) {
            engine.act(slot(s), action).expect("an accepted action");
        }
    }
    engine.step(&minute_inputs(tick));
}

/// The three-player equipment run at tick 0.
pub fn start() -> Engine {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: SEED,
        ..RunSetup::default()
    };
    Engine::start_party_with(&setup, &worst_minute_case::content(), 3)
        .expect("the three-player equipment run starts")
}

/// The three-player equipment run, `ticks` ticks in.
fn fast_forwarded(ticks: u32) -> Engine {
    let mut engine = start();
    for _ in 0..ticks {
        play_tick(&mut engine);
    }
    engine
}

/// The engine the rollback restores: the run `EQUIP_PEAK_TICK` ticks in, with no action pending.
pub fn saved() -> Engine {
    fast_forwarded(EQUIP_PEAK_TICK)
}

/// The measured body of the rollback bench: a clone of `saved`, `ROLLBACK_TICKS` ticks played on
/// it with the actions of `ROLLBACK_ACTIONS`, and both engines handed back so that nothing is
/// dropped inside the counted region.
#[inline]
pub fn rollback_equip(saved: Engine) -> (Engine, Engine) {
    let mut engine = saved.clone();
    for i in 0..ROLLBACK_TICKS {
        for (at, actions) in &ROLLBACK_ACTIONS {
            if *at == i {
                for (s, action) in (0u8..).zip(actions) {
                    if let Some(action) = action {
                        engine.act(slot(s), *action).expect("an accepted action");
                    }
                }
            }
        }
        engine.step(&minute_inputs(engine.tick()));
    }
    (engine, saved)
}

fn weight(load: Load) -> u64 {
    [
        load.enemies,
        load.tasks,
        load.zones,
        load.orbs,
        load.emitter_steps,
    ]
    .iter()
    .map(|&n| u64::from(n))
    .sum()
}

/// Whether every present slot holds at least two items.
fn stocked(engine: &Engine) -> bool {
    engine
        .snapshot()
        .players
        .iter()
        .all(|p| p.inventory.len() >= 2)
}

/// Scans the dense minute for its heaviest tick that starts with every inventory at two or
/// more (the first wins a tie): the minute's start and the number of ticks played before it.
pub fn window() -> (u32, u32) {
    let mut engine = fast_forwarded(MINUTE_START);
    let mut heaviest = (0, 0);
    for i in 0..MINUTE_TICKS {
        let ready = stocked(&engine);
        play_tick(&mut engine);
        let w = weight(engine.load());
        if ready && w > heaviest.0 {
            heaviest = (w, MINUTE_START + i);
        }
    }
    assert!(heaviest.0 > 0, "a tick with every inventory stocked");
    (MINUTE_START, heaviest.1)
}
