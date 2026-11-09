//! Shared test support for the net crate: the run every test starts from, a scripted bot, and an
//! oracle that steps a straight engine with no session in between. Each test binary uses part of
//! it, so unused items are allowed.
#![allow(dead_code)]

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_setup::{BuildMode, RunSetup};

/// The run every net test starts: the trial with the giant sword, in equipment mode. This is the
/// only place a test names the build mode.
pub fn equipment_setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    }
}

/// An engine of `n` players (1 to 3) at tick 0 on [`equipment_setup`].
pub fn engine(n: u8) -> Engine {
    Engine::start_party(&equipment_setup(), n).expect("the equipment setup starts")
}

/// The slot `i`.
pub fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("slot 0 to 2")
}

/// A small deterministic generator (xorshift64*).
pub struct Xorshift(pub u64);

impl Xorshift {
    /// A generator from `seed`; a zero seed is replaced.
    pub fn new(seed: u64) -> Xorshift {
        Xorshift(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 bits.
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `bound` (not zero).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// What every peer is to play: the effective input of each slot at each tick, and the ticks at
/// which a slot leaves. `eff[t]` is stepped at tick `t`; its `action` is made before the step.
#[derive(Clone, Debug)]
pub struct Script {
    /// The party size the script was made for.
    pub n: u8,
    /// The input of every slot at every tick.
    pub eff: Vec<[Input; 3]>,
    /// `(tick, slot)`: the slot leaves before tick `tick` is stepped.
    pub leaves: Vec<(u32, PlayerIdx)>,
}

impl Script {
    /// The number of ticks the script covers.
    pub fn ticks(&self) -> u32 {
        u32::try_from(self.eff.len()).expect("a short script")
    }
}

/// Applies one tick the way a recorder does: the leaves stamped at `tick` (slot order), the
/// actions in slot order (a refusal is ignored), then the step on inputs with `action` cleared.
pub fn apply_tick(
    engine: &mut Engine,
    tick: u32,
    inputs: &[Input; 3],
    leaves: &[(u32, PlayerIdx)],
) {
    let mut gone = [false; 3];
    for (t, who) in leaves {
        if *t == tick {
            gone[who.get()] = true;
        }
    }
    for (i, g) in gone.iter().enumerate() {
        if *g {
            let _ = engine.leave(slot(u8::try_from(i).expect("small")));
        }
    }
    for (i, input) in inputs.iter().enumerate() {
        let who = slot(u8::try_from(i).expect("small"));
        if engine.present() >> who.get() & 1 == 1
            && let Some(action) = input.action
        {
            let _ = engine.act(who, action);
        }
    }
    let mut cleared = *inputs;
    for input in &mut cleared {
        input.action = None;
    }
    engine.step(&cleared);
}

/// Every equipment action this bot would try, in a fixed order.
fn candidates() -> Vec<EquipAction> {
    let mut all = vec![EquipAction::TreePick { option: 0 }];
    for item in 0..4 {
        for slot in 0..6 {
            all.push(EquipAction::Equip { item, slot });
            all.push(EquipAction::Replace { item, slot });
        }
    }
    all.push(EquipAction::Extract { item: 0 });
    for star in 0..8 {
        all.push(EquipAction::Craft { star });
    }
    all
}

/// The bot's input for `who` at the tick `engine` is on: a wandering stick, held buttons now and
/// then, and every few ticks an equipment action the engine accepts (chosen among those it
/// accepts, so the kinds vary). Stick axes stay within -127 to 127 (the wire form).
fn bot_input(engine: &Engine, who: PlayerIdx, tick: u32, rng: &mut Xorshift) -> Input {
    let leg = tick / 24;
    let mut leg_rng =
        Xorshift::new(u64::from(leg) * 3 + u64::try_from(who.get()).expect("small") + 1);
    leg_rng.next();
    let axis = |r: &mut Xorshift| {
        i8::try_from(i64::try_from(r.below(255)).expect("small") - 127).expect("-127 to 127")
    };
    let mut input = Input {
        move_x: axis(&mut leg_rng),
        move_y: axis(&mut leg_rng),
        main: tick % 3 != 0,
        smash: tick % 50 > 44,
        evade: tick % 90 == 5,
        skill: tick % 70 == 30,
        assist: leg % 2 == 0,
        ..Input::default()
    };
    if tick % 5 == 0 {
        let accepted: Vec<EquipAction> = candidates()
            .into_iter()
            .filter(|a| engine.check_action(who, *a).is_ok())
            .collect();
        if !accepted.is_empty() {
            let pick = usize::try_from(rng.below(accepted.len() as u64)).expect("small");
            input.action = Some(accepted[pick]);
        }
    }
    input
}

/// Plays `n` players for `ticks` ticks with the bot and records the effective inputs. Ticks
/// below `delay` are neutral (every peer's first inputs are); a slot's inputs from its leave tick
/// on are neutral. `leave` is `(slot, tick)`.
pub fn script(n: u8, ticks: u32, delay: u32, leave: Option<(PlayerIdx, u32)>) -> Script {
    let leaves: Vec<(u32, PlayerIdx)> = leave.into_iter().map(|(who, t)| (t, who)).collect();
    let mut e = engine(n);
    let mut rng = Xorshift::new(0xC0FF_EE11);
    let mut eff = Vec::new();
    for t in 0..ticks {
        let mut row = [Input::default(); 3];
        if t >= delay {
            for i in 0..n {
                let who = slot(i);
                let left = leaves.iter().any(|(lt, w)| *w == who && *lt <= t);
                if !left && e.present() >> i & 1 == 1 {
                    row[usize::from(i)] = bot_input(&e, who, t, &mut rng);
                }
            }
        }
        apply_tick(&mut e, t, &row, &leaves);
        eff.push(row);
    }
    Script { n, eff, leaves }
}

/// The oracle: a fresh engine of `n` players stepped through the script, no session involved.
/// `hashes[k]` is the state hash after `k` ticks, so the vector is one longer than the script.
pub fn straight(script: &Script, n: u8) -> Vec<u64> {
    let mut e = engine(n);
    let mut hashes = vec![e.state_hash()];
    for (t, row) in script.eff.iter().enumerate() {
        let t = u32::try_from(t).expect("short");
        apply_tick(&mut e, t, row, &script.leaves);
        hashes.push(e.state_hash());
    }
    hashes
}
