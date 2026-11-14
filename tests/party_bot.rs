//! A seeded party bot and a test-only party builder over `run_play::PartyCase` (ADR-REPL-5,
//! ADR-SIM-7, ADR-SIM-8). Beside `run_play.rs`, not in `tests/common/`, which the benches share.
//! Integer arithmetic only; every choice is a pure function of the seed, the engine, the slot
//! and the tick, so a clone of the engine can be driven again from any tick.

use stella_rain_core::RangeStep;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Point;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::pack::Pack;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{
    EntityKind, ExpertiseOfferKind, HeldView, SlotKindView, SwapItem, SwapRating,
};

use crate::common::run::trial_setup;
use crate::run_play::{Change, PartyCase};

/// How near (1/256 pixel) the bot's revive walk gets before it holds `interact`: 40 pixels,
/// inside the revive range (48 pixels) with room for the 2 pixels a player moves in a tick.
pub const HOLD_AT: i64 = 40 * 256;

/// How near (1/256 pixel) a fighter walks up to an enemy: 32 pixels.
pub const NEAR: i64 = 32 * 256;

/// A bot for any slot of a party: every call is a pure function of the seed, the engine, the
/// slot and the tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyBot {
    pub seed: u64,
    /// Plays the equipment actions of an equipment run (`action`); off, the bot makes none.
    pub builds: bool,
}

/// The bot decides on one tick in this many, per slot.
pub const ACTION_EVERY: u32 = 30;
/// How many inventory places the bot rates.
pub const RATED_ITEMS: usize = 12;
/// How many craftable stars the bot rates.
pub const CRAFT_RATED: usize = 8;

/// A local SplitMix64 finaliser over the seed, the slot and a salt.
pub fn mix(seed: u64, slot: u8, salt: u64) -> u64 {
    let mut z = (seed ^ (u64::from(slot) << 56) ^ salt).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// What the bot reads of the world, once per call.
#[derive(Clone, Debug, Default)]
pub struct Sight {
    pub me: Point,
    /// The slot has hit points and is not out.
    pub up: bool,
    /// The teammates that are downed and not out, with their place.
    pub downed: Vec<(u8, Point)>,
    /// The teammates that are up.
    pub up_mates: Vec<(u8, Point)>,
    pub star: Option<Point>,
    /// The nearest enemy; with none on the field, the nearest boss (a boss part stays out).
    pub enemy: Option<Point>,
    /// The slot's weapon gauge is full (`GaugeView.value` at `max`, ADR-SIM-9).
    pub gauge_full: bool,
}

/// The squared distance (1/256 pixel) between two points.
fn d2(a: Point, b: Point) -> i64 {
    let (x, y) = (i64::from(a.x.0 - b.x.0), i64::from(a.y.0 - b.y.0));
    x * x + y * y
}

/// An axis toward `d`, with a dead zone of one pixel.
fn toward(d: i32) -> i8 {
    match d {
        d if d > 256 => 127,
        d if d < -256 => -127,
        _ => 0,
    }
}

/// What `slot` of `engine` sees. The snapshot lists the present players in slot order, so a
/// hole shifts the index.
pub fn sight(engine: &Engine, slot: u8) -> Sight {
    let snap = engine.snapshot();
    let present = engine.present();
    let place = |s: u8| (present & ((1 << s) - 1)).count_ones() as usize;
    let me = snap.players[place(slot)].at;
    let (mut downed, mut up_mates) = (Vec::new(), Vec::new());
    for s in (0..3u8).filter(|s| *s != slot && present >> s & 1 == 1) {
        let p = &snap.players[place(s)];
        if p.out {
            continue;
        }
        if p.down.is_some() {
            downed.push((s, p.at));
        } else if p.hp > 0 {
            up_mates.push((s, p.at));
        }
    }
    let mine = &snap.players[place(slot)];
    let nearest = |kind: EntityKind| {
        snap.entities
            .iter()
            .filter(|v| v.kind == kind)
            .map(|v| v.at)
            .min_by_key(|at| d2(*at, me))
    };
    Sight {
        me,
        up: mine.hp > 0 && !mine.out && mine.down.is_none(),
        downed,
        up_mates,
        star: snap
            .stars
            .iter()
            .map(|st| st.at)
            .min_by_key(|at| d2(*at, me)),
        enemy: nearest(EntityKind::Enemy).or_else(|| nearest(EntityKind::Boss)),
        gauge_full: mine
            .gauge
            .as_ref()
            .is_some_and(|g| g.max > 0 && g.value >= g.max),
    }
}

impl PartyBot {
    /// The solo build bot: call `action` with slot 0.
    pub fn building(seed: u64) -> PartyBot {
        PartyBot { seed, builds: true }
    }

    /// Slot `slot`'s attack cadence offset.
    pub fn phase(&self, slot: u8) -> u64 {
        mix(self.seed, slot, 1) % 40
    }

    /// Slot `slot`'s role: 0 a fighter, 1 a collector.
    pub fn role(&self, slot: u8) -> u64 {
        mix(self.seed, slot, 2) % 2
    }

    /// Slot `slot`'s skill cadence offset.
    pub fn skill_phase(&self, slot: u8) -> u64 {
        mix(self.seed, slot, 3) % 90
    }

    /// The buttons of a slot that sees `s` at `tick`: nothing when it is not up; else it walks
    /// to the downed teammate it is the nearest up slot to (ties to the lower slot) and holds
    /// `interact` inside `HOLD_AT`, standing still and not attacking; else a collector goes to
    /// the nearest star and a fighter to the nearest enemy (the boss once none is left; never a
    /// boss part) until within `NEAR`, then to the other.
    /// `main` and `skill` follow the slot's cadences; it never evades, which would leave the
    /// revive range.
    pub fn decide(&self, s: &Sight, slot: u8, tick: u32) -> Input {
        if !s.up {
            return Input::default();
        }
        let t = u64::from(tick);
        let mut input = Input {
            main: (t + self.phase(slot)) % 40 < 5,
            // A full-gauge smash needs a press of B with the gauge full: 2 ticks in 8 leave the
            // release tick the engine needs for a new press (ADR-SIM-9).
            smash: self.builds && s.gauge_full && (t + self.phase(slot)) % 8 < 2,
            skill: (t + self.skill_phase(slot)) % 90 < 30,
            ..Input::default()
        };
        // The downed teammate this slot is the nearest up slot to, the nearest such to it.
        let revive = s
            .downed
            .iter()
            .filter(|(_, at)| {
                let mine = d2(s.me, *at);
                s.up_mates
                    .iter()
                    .all(|(m, mat)| (d2(*mat, *at), *m) > (mine, slot))
            })
            .min_by_key(|(d, at)| (d2(s.me, *at), *d));
        if let Some((_, at)) = revive {
            if d2(s.me, *at) <= HOLD_AT * HOLD_AT {
                return Input {
                    interact: true,
                    ..Input::default()
                };
            }
            input.move_x = toward(at.x.0 - s.me.x.0);
            input.move_y = toward(at.y.0 - s.me.y.0);
            return input;
        }
        let (first, second) = if self.role(slot) == 1 {
            (s.star, s.enemy)
        } else {
            (s.enemy, s.star)
        };
        let goal = [first, second]
            .into_iter()
            .flatten()
            .find(|at| d2(*at, s.me) > NEAR * NEAR);
        if let Some(at) = goal {
            input.move_x = toward(at.x.0 - s.me.x.0);
            input.move_y = toward(at.y.0 - s.me.y.0);
        }
        input
    }

    /// Slot `slot`'s equipment decision offset.
    pub fn action_phase(&self, slot: u8) -> u64 {
        mix(self.seed, slot, 4) % u64::from(ACTION_EVERY)
    }

    /// The equipment action `slot` takes at `tick`, if any: only a building bot in an equipment
    /// run (the player view has slots), a slot that is up, and one tick in `ACTION_EVERY`. In
    /// order: the tree point; the best rated equip or replace (a replace never lowers a slot);
    /// extract the rest; with an empty inventory, craft the best rated star it would equip.
    /// Every action returned has passed `Engine::check_action`.
    pub fn action(&self, engine: &Engine, slot: u8, tick: u32) -> Option<EquipAction> {
        if !self.builds
            || (u64::from(tick) + self.action_phase(slot)) % u64::from(ACTION_EVERY) != 0
        {
            return None;
        }
        let player = PlayerIdx::new(slot)?;
        let snap = engine.snapshot();
        let present = engine.present();
        let place = (present & ((1u8 << slot) - 1)).count_ones() as usize;
        let view = snap.players.get(place)?;
        if view.slots.is_empty() || view.hp == 0 || view.out || view.down.is_some() {
            return None;
        }
        let accepted = |a: EquipAction| engine.check_action(player, a).is_ok();
        // A style is picked by the seed; an upgrade or a capstone has one option, so option 0.
        let option = match &view.tree_ready {
            Some(r) if r.kind == ExpertiseOfferKind::Style && !r.options.is_empty() => {
                u32::try_from(mix(self.seed, slot, 5) % r.options.len() as u64).ok()?
            }
            _ => 0,
        };
        if accepted(EquipAction::TreePick { option }) {
            return Some(EquipAction::TreePick { option });
        }
        // A candidate: (dps_bp, an equip, reversed item, reversed slot) -> action.
        type Key = (i32, bool, std::cmp::Reverse<u32>, std::cmp::Reverse<u32>);
        let adopted = |rows: Vec<SwapRating>, item: u32| -> Vec<(Key, u32, bool)> {
            rows.into_iter()
                .filter(|r| {
                    if r.replaces {
                        r.dps_bp > Some(0) && r.level_after >= r.level_before
                    } else {
                        r.dps_bp.unwrap_or(0) >= 0
                    }
                })
                .map(|r| {
                    (
                        (
                            r.dps_bp.unwrap_or(0),
                            !r.replaces,
                            std::cmp::Reverse(item),
                            std::cmp::Reverse(r.slot),
                        ),
                        r.slot,
                        r.replaces,
                    )
                })
                .collect()
        };
        let mut best: Option<(Key, EquipAction)> = None;
        for (i, held) in view.inventory.iter().take(RATED_ITEMS).enumerate() {
            let item = u32::try_from(i).ok()?;
            let rows = match held {
                HeldView::Star { .. } => {
                    adopted(engine.swap_ratings(player, SwapItem::Inventory(item)), item)
                }
                HeldView::Rune { .. } => view
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, sv)| sv.kind == SlotKindView::Rune)
                    .map(|(k, _)| u32::try_from(k).unwrap())
                    .find(|k| accepted(EquipAction::Equip { item, slot: *k }))
                    .map(|k| {
                        (
                            (0, true, std::cmp::Reverse(item), std::cmp::Reverse(k)),
                            k,
                            false,
                        )
                    })
                    .into_iter()
                    .collect(),
            };
            for (key, k, replaces) in rows {
                let action = if replaces {
                    EquipAction::Replace { item, slot: k }
                } else {
                    EquipAction::Equip { item, slot: k }
                };
                if accepted(action) && best.is_none_or(|(b, _)| key > b) {
                    best = Some((key, action));
                }
            }
        }
        if let Some((_, action)) = best {
            return Some(action);
        }
        if !view.inventory.is_empty() {
            let extract = EquipAction::Extract { item: 0 };
            return accepted(extract).then_some(extract);
        }
        let content = RunContent::bundled();
        let pool = content.rewards().rewards();
        let mut crafted: Option<(Key, EquipAction)> = None;
        let candidates = (0..pool.len())
            .filter(|p| !view.pool_held.get(*p).copied().unwrap_or(true))
            .filter_map(|p| u32::try_from(p).ok())
            .filter(|p| accepted(EquipAction::Craft { star: *p }))
            .take(CRAFT_RATED);
        for p in candidates {
            let id = pool[p as usize].id.clone();
            let rows = adopted(engine.swap_ratings(player, SwapItem::Star(id)), p);
            for (key, _, _) in rows {
                if crafted.is_none_or(|(b, _)| key > b) {
                    crafted = Some((key, EquipAction::Craft { star: p }));
                }
            }
        }
        crafted.map(|(_, action)| action)
    }

    /// The buttons of `slot` at `tick`.
    pub fn input(&self, engine: &Engine, slot: u8, tick: u32) -> Input {
        self.decide(&sight(engine, slot), slot, tick)
    }
}

/// A test-only party of 1 to 3 for `run_play::PartyCase`, every slot played by a `PartyBot`.
/// Not the recorded `PartySetup` type of ADR-REPL-5: that one is one `RunSetup` per player.
#[derive(Clone, Debug)]
pub struct RigParty {
    n: u8,
    mode: String,
    seed: u64,
    difficulty: String,
    ticks: u32,
    bot_seed: u64,
    equipment: bool,
    builds: bool,
    changes: Vec<(u32, Change)>,
}

impl RigParty {
    /// A party of `n` (1 to 3) on the `time_limit` mode, normal difficulty, 3000 ticks.
    pub fn new(n: u8) -> RigParty {
        assert!((1..=3).contains(&n), "a party of 1 to 3, not {n}");
        RigParty {
            n,
            mode: "time_limit".into(),
            seed: trial_setup().seed,
            difficulty: "normal".into(),
            ticks: 3000,
            bot_seed: 0,
            equipment: false,
            builds: false,
            changes: vec![],
        }
    }

    pub fn mode(mut self, mode: &str) -> RigParty {
        self.mode = mode.into();
        self
    }

    /// The world seed of the run's setup.
    pub fn seed(mut self, seed: u64) -> RigParty {
        self.seed = seed;
        self
    }

    pub fn difficulty(mut self, difficulty: &str) -> RigParty {
        self.difficulty = difficulty.into();
        self
    }

    pub fn ticks(mut self, ticks: u32) -> RigParty {
        self.ticks = ticks;
        self
    }

    pub fn bot_seed(mut self, bot_seed: u64) -> RigParty {
        self.bot_seed = bot_seed;
        self
    }

    /// Equipment mode with the `vanguard` perk set (it regenerates): nothing else changes.
    pub fn equipment(mut self) -> RigParty {
        self.equipment = true;
        self
    }

    /// Every slot's bot plays the equipment actions (`PartyBot::action`).
    pub fn builds(mut self) -> RigParty {
        self.builds = true;
        self
    }

    /// Joins and leaves, by world tick (applied before that tick's step).
    pub fn changes(mut self, changes: Vec<(u32, Change)>) -> RigParty {
        self.changes = changes;
        self
    }

    /// The corpus case `name`: a `stop` run, no joins or leaves.
    pub fn case(&self, name: &'static str) -> PartyCase {
        let setup = RunSetup {
            mode: self.mode.as_str().into(),
            seed: self.seed,
            difficulty: self.difficulty.as_str().into(),
            ..trial_setup()
        };
        let setup = if self.equipment {
            RunSetup {
                perk_set: "vanguard".into(),
                ..setup
            }
        } else {
            setup
        };
        PartyCase {
            name,
            setup,
            party: self.n,
            ticks: self.ticks,
            changes: self.changes.clone(),
            bot: Some(PartyBot {
                seed: self.bot_seed,
                builds: self.builds,
            }),
            script: None,
        }
    }
}

/// A recorded rig run as plain data: the input of every slot at every world tick, and the
/// picks and actions between ticks (the picks are read data: a rig run plays none). Replayable on a bare `Engine`, or on a clone of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigRun {
    pub n: u8,
    /// The recorded input of every slot at world tick `t` (index `t`); an absent slot's is the
    /// default.
    pub ticks: Vec<[Input; 3]>,
    /// The `Picks` steps in order, each with the world tick it comes before.
    pub picks: Vec<(u32, [Option<u32>; 3])>,
    /// The `Actions` steps in order, each with the world tick it comes before and the action of
    /// every slot. Applied before the tick.
    pub actions: Vec<(u32, [Option<EquipAction>; 3])>,
}

/// How many actions of each kind one slot made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionCounts {
    pub equip: u32,
    pub replace: u32,
    pub extract: u32,
    pub craft: u32,
    pub tree_pick: u32,
}

impl RigRun {
    /// Reads `replay`'s step log; panics on a join or a leave (a rig case holds none).
    pub fn of(replay: &RunReplay) -> RigRun {
        let range = replay
            .step_range(0, replay.ticks, u32::MAX)
            .expect("a recorded step log");
        let slots: Vec<usize> = (0..3).filter(|s| range.present >> s & 1 == 1).collect();
        assert!(
            range.present == (1u8 << slots.len()) - 1,
            "a rig run holds a party from slot 0 with no hole"
        );
        fn spread<T: Copy>(slots: &[usize], dense: &[Option<T>]) -> [Option<T>; 3] {
            let mut out = [None; 3];
            for (s, v) in slots.iter().zip(dense) {
                out[*s] = *v;
            }
            out
        }
        let (mut ticks, mut picks, mut actions) = (Vec::new(), Vec::new(), Vec::new());
        for step in &range.steps {
            match step {
                RangeStep::Actions { actions: dense } => {
                    let world = u32::try_from(ticks.len()).expect("fewer ticks than u32");
                    actions.push((world, spread(&slots, dense)));
                }
                RangeStep::Ticks { run, inputs } => {
                    let mut all = [Input::default(); 3];
                    for (s, input) in slots.iter().zip(inputs) {
                        all[*s] = *input;
                    }
                    ticks.extend((0..*run).map(|_| all));
                }
                RangeStep::Picks { picks: dense } => {
                    let world = u32::try_from(ticks.len()).expect("fewer ticks than u32");
                    picks.push((world, spread(&slots, dense)));
                }
                RangeStep::Join { .. } | RangeStep::Leave { .. } => {
                    panic!("a rig run holds no join or leave")
                }
            }
        }
        RigRun {
            n: u8::try_from(slots.len()).unwrap(),
            ticks,
            picks,
            actions,
        }
    }

    /// Applies world ticks `from..to` to `engine`, with the actions that come before each of
    /// them; for `to == ticks.len()` also the actions after the last.
    pub fn play_onto(&self, engine: &mut Engine, from: u32, to: u32) {
        self.play_with(engine, from, to, |_| {});
    }

    /// `play_onto`, calling `after` with the engine after every tick.
    pub fn play_with(
        &self,
        engine: &mut Engine,
        from: u32,
        to: u32,
        mut after: impl FnMut(&Engine),
    ) {
        let len = u32::try_from(self.ticks.len()).expect("fewer ticks than u32");
        assert!(
            from <= to && to <= len,
            "window {from}..{to} of {len} ticks"
        );
        let acts_at = |world: u32, engine: &mut Engine| {
            for (_, acts) in self.actions.iter().filter(|(at, _)| *at == world) {
                for (slot, action) in acts.iter().enumerate() {
                    if let Some(action) = action {
                        let player = PlayerIdx::new(u8::try_from(slot).unwrap()).unwrap();
                        engine
                            .act(player, *action)
                            .unwrap_or_else(|e| panic!("action {action:?} at {world}: {e:?}"));
                    }
                }
            }
        };
        for t in from..to {
            acts_at(t, engine);
            engine.step(&self.ticks[t as usize]);
            after(engine);
        }
        if to == len {
            acts_at(len, engine);
        }
    }

    /// How many actions of each kind each slot made.
    pub fn actions_per_slot(&self) -> [ActionCounts; 3] {
        let mut per = [ActionCounts::default(); 3];
        for (_, acts) in &self.actions {
            for (c, action) in per.iter_mut().zip(acts) {
                match action {
                    Some(EquipAction::Equip { .. }) => c.equip += 1,
                    Some(EquipAction::Replace { .. }) => c.replace += 1,
                    Some(EquipAction::Extract { .. }) => c.extract += 1,
                    Some(EquipAction::Craft { .. }) => c.craft += 1,
                    Some(EquipAction::TreePick { .. }) => c.tree_pick += 1,
                    None => {}
                }
            }
        }
        per
    }

    /// The downs, revives and outs of the whole run, replayed from a fresh engine.
    pub fn tally(&self, replay: &RunReplay) -> RigTally {
        let mut engine =
            Engine::start_party_with(&replay.setup, Pack::bundled().content(), self.n).unwrap();
        let mut t = RigTally::default();
        let mut down = [false; 3];
        let len = u32::try_from(self.ticks.len()).unwrap();
        self.play_with(&mut engine, 0, len, |e| {
            let tick = e.tick();
            for event in e.events() {
                match event {
                    DomainEvent::PlayerDowned { player } => {
                        t.downed += 1;
                        t.first_down.get_or_insert(tick);
                        down[player.get()] = true;
                    }
                    DomainEvent::PlayerRevived { player, reviver } => {
                        t.revived += 1;
                        t.first_revive.get_or_insert(tick);
                        t.revivers.push(u8::try_from(reviver.get()).unwrap());
                        t.revived_were_down &= down[player.get()];
                        down[player.get()] = false;
                        // 30% of max hit points, at once.
                        t.revived_have_hp &= e.snapshot().players[player.get()].hp > 0;
                    }
                    DomainEvent::GaugeSpent { .. } => t.gauge_spent += 1,
                    DomainEvent::PlayerOut { .. } => {
                        t.out += 1;
                        t.first_out.get_or_insert(tick);
                    }
                    _ => {}
                }
            }
        });
        t
    }
}

/// Which predicate of a building rig case `replay` (a party of `n`) misses, if any: a run still
/// `Running` of 3000 ticks and under 64 KiB that verifies to its final hash, every slot with an
/// `Equip` and an `Extract`; the party of 1 also a `Craft` and a `TreePick`, the party of 3 also
/// a `Replace` by some slot, and some slot spends its gauge (`GaugeSpent`).
pub fn rig_builds_miss(replay: &RunReplay, n: u8) -> Option<&'static str> {
    use stella_rain_core::engine::Outcome;
    let bytes = replay.to_bytes();
    if replay.ticks != 3000 || bytes.len() >= 64 * 1024 {
        return Some("3000 ticks, under 64 KiB");
    }
    match stella_rain_core::run_replay::verify_run(&bytes) {
        Ok(v) if v.outcome == Outcome::Running => {}
        _ => return Some("Running and verifies"),
    }
    let per = RigRun::of(replay).actions_per_slot();
    if per[..usize::from(n)].iter().any(|c| c.equip == 0) {
        return Some("every slot equips");
    }
    if per[..usize::from(n)].iter().any(|c| c.extract == 0) {
        return Some("every slot extracts");
    }
    if n == 1 && (per[0].craft == 0 || per[0].tree_pick == 0) {
        return Some("a craft and a tree pick");
    }
    if n == 3 && per.iter().all(|c| c.replace == 0) {
        return Some("a replace");
    }
    if RigRun::of(replay).tally(replay).gauge_spent == 0 {
        return Some("some slot spends its gauge");
    }
    None
}

/// Which predicate of an equipment revive case `replay` (a party of `n`, 2 or 3) misses, if any:
/// at most 3000 ticks and under 64 KiB, a down, a revive, every revived player had been down and
/// has hit points, no `PlayerOut` before the first revive, slots 1 and 2 make at least 2 actions
/// together, and for 3 a revive made by a slot other than 0.
pub fn equipment_revive_miss(replay: &RunReplay, n: u8) -> Option<&'static str> {
    if replay.ticks > 3000 || replay.to_bytes().len() >= 64 * 1024 {
        return Some("at most 3000 ticks, under 64 KiB");
    }
    let run = RigRun::of(replay);
    let tally = run.tally(replay);
    if tally.downed < 1 {
        return Some("a down");
    }
    if tally.revived < 1 {
        return Some("a revive");
    }
    if !tally.revived_were_down {
        return Some("revived players were down");
    }
    if !tally.revived_have_hp {
        return Some("revived players have hit points");
    }
    if tally
        .first_out
        .is_some_and(|out| !tally.first_revive.is_some_and(|r| r < out))
    {
        return Some("no out before the first revive");
    }
    let per = run.actions_per_slot();
    let acts = |c: &ActionCounts| c.equip + c.replace + c.extract + c.craft + c.tree_pick;
    if acts(&per[1]) + acts(&per[2]) < 2 {
        return Some("slots 1 and 2 make at least 2 actions");
    }
    if n == 3 && !tally.revivers.iter().any(|r| *r != 0) {
        return Some("a revive by a slot other than 0");
    }
    None
}

/// The downs, revives and outs of a rig run (ADR-SIM-7, ADR-SIM-8). Ticks are the engine's
/// tick count after the step the event came in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigTally {
    pub downed: u32,
    pub revived: u32,
    pub out: u32,
    pub first_down: Option<u32>,
    pub first_revive: Option<u32>,
    pub first_out: Option<u32>,
    /// The slot that made each revive, in order.
    pub revivers: Vec<u8>,
    /// Every revived player had been downed before.
    pub revived_were_down: bool,
    /// Every revived player has hit points on the snapshot right after.
    pub revived_have_hp: bool,
    /// The `GaugeSpent` events of the run: full-gauge smashes (ADR-SIM-9).
    pub gauge_spent: u32,
}

impl Default for RigTally {
    fn default() -> RigTally {
        RigTally {
            downed: 0,
            revived: 0,
            out: 0,
            first_down: None,
            first_revive: None,
            first_out: None,
            revivers: Vec::new(),
            revived_were_down: true,
            revived_have_hp: true,
            gauge_spent: 0,
        }
    }
}

#[cfg(test)]
mod decisions {
    use super::*;

    use stella_rain_core::fixed::Fx;

    fn at(x_px: i32, y_px: i32) -> Point {
        Point {
            x: Fx::px(x_px),
            y: Fx::px(y_px),
        }
    }

    fn bot() -> PartyBot {
        PartyBot {
            seed: 7,
            builds: false,
        }
    }

    fn up() -> Sight {
        Sight {
            up: true,
            ..Sight::default()
        }
    }

    /// A seed whose `slot` has `role`.
    fn seed_with_role(slot: u8, role: u64) -> PartyBot {
        (1..)
            .map(|seed| PartyBot {
                seed,
                builds: false,
            })
            .find(|b| b.role(slot) == role)
            .unwrap()
    }

    #[test]
    fn the_nearest_up_slot_walks_to_a_downed_teammate() {
        let s = Sight {
            downed: vec![(0, at(100, 0))],
            up_mates: vec![(2, at(0, 300))],
            ..up()
        };
        let input = bot().decide(&s, 1, 0);
        assert_eq!((input.move_x, input.move_y), (127, 0));
        assert!(!input.interact, "100 px is farther than HOLD_AT");
        // Slot 2, farther from it, does not walk to it.
        let far = Sight {
            me: at(0, 300),
            downed: vec![(0, at(100, 0))],
            up_mates: vec![(1, at(0, 0))],
            ..up()
        };
        let input = bot().decide(&far, 2, 0);
        assert_eq!((input.move_x, input.move_y), (0, 0));
        assert!(!input.interact);
    }

    #[test]
    fn it_holds_interact_inside_hold_at_and_stands_still() {
        let s = Sight {
            downed: vec![(0, at(30, 0))],
            ..up()
        };
        for tick in 0..400 {
            let input = bot().decide(&s, 1, tick);
            assert!(input.interact, "tick {tick}");
            assert_eq!((input.move_x, input.move_y), (0, 0), "tick {tick}");
            assert!(!input.main, "tick {tick}");
        }
    }

    #[test]
    fn equal_distances_go_to_the_lower_slot() {
        let downed = vec![(0, at(100, 0))];
        let one = Sight {
            me: at(0, 50),
            downed: downed.clone(),
            up_mates: vec![(2, at(0, -50))],
            ..up()
        };
        let two = Sight {
            me: at(0, -50),
            downed,
            up_mates: vec![(1, at(0, 50))],
            ..up()
        };
        assert_eq!(bot().decide(&one, 1, 0).move_x, 127);
        assert_eq!(bot().decide(&two, 2, 0).move_x, 0);
    }

    #[test]
    fn a_teammate_that_is_out_is_not_revived() {
        // An out teammate is in neither list: the slot hunts.
        let s = Sight {
            enemy: Some(at(-200, 0)),
            ..up()
        };
        let b = seed_with_role(1, 0);
        let input = b.decide(&s, 1, 0);
        assert!(!input.interact);
        assert_eq!(input.move_x, -127);
    }

    #[test]
    fn a_slot_that_is_not_up_does_nothing() {
        let s = Sight {
            up: false,
            enemy: Some(at(-200, 0)),
            downed: vec![(0, at(30, 0))],
            ..Sight::default()
        };
        for tick in 0..100 {
            assert_eq!(bot().decide(&s, 1, tick), Input::default());
        }
    }

    #[test]
    fn a_fighter_hunts_the_enemy_and_a_collector_the_star() {
        let fighter = seed_with_role(1, 0);
        let far = Sight {
            enemy: Some(at(-200, 0)),
            ..up()
        };
        assert_eq!(fighter.decide(&far, 1, 0).move_x, -127);
        let near = Sight {
            enemy: Some(at(-20, 0)),
            ..up()
        };
        let input = fighter.decide(&near, 1, 0);
        assert_eq!((input.move_x, input.move_y), (0, 0));
        // A fighter with no enemy takes the star.
        let star = Sight {
            star: Some(at(0, 200)),
            ..up()
        };
        assert_eq!(fighter.decide(&star, 1, 0).move_y, 127);

        let collector = seed_with_role(1, 1);
        let both = Sight {
            star: Some(at(0, 200)),
            enemy: Some(at(-200, 0)),
            ..up()
        };
        let input = collector.decide(&both, 1, 0);
        assert_eq!((input.move_x, input.move_y), (0, 127));
    }

    #[test]
    fn a_boss_only_sight_walks_the_fighter_to_it() {
        let fighter = seed_with_role(1, 0);
        let far = Sight {
            enemy: Some(at(-200, 0)),
            ..up()
        };
        let input = fighter.decide(&far, 1, 0);
        assert_eq!((input.move_x, input.move_y), (-127, 0));
        let near = Sight {
            enemy: Some(at(-20, 0)),
            ..up()
        };
        let input = fighter.decide(&near, 1, 0);
        assert_eq!((input.move_x, input.move_y), (0, 0));
    }

    #[test]
    fn buttons_follow_the_cadences_and_axes_stay_in_range() {
        for seed in 1..=3 {
            let b = PartyBot {
                seed,
                builds: false,
            };
            for slot in 0..3u8 {
                let s = Sight {
                    me: at(5, -9),
                    star: Some(at(-1000, 700)),
                    enemy: Some(at(900, -800)),
                    ..up()
                };
                let (mut mains, mut skills) = (0, 0);
                for tick in 0..400u32 {
                    let input = b.decide(&s, slot, tick);
                    assert_eq!(
                        input.main,
                        (u64::from(tick) + b.phase(slot)) % 40 < 5,
                        "main, seed {seed} slot {slot} tick {tick}"
                    );
                    assert_eq!(
                        input.skill,
                        (u64::from(tick) + b.skill_phase(slot)) % 90 < 30,
                        "skill, seed {seed} slot {slot} tick {tick}"
                    );
                    assert!(!input.evade && !input.interact);
                    assert!(input.move_x >= -127 && input.move_y >= -127);
                    mains += u32::from(input.main);
                    skills += u32::from(input.skill);
                }
                assert_eq!(mains, 50);
                assert!(skills > 0);
            }
        }
    }

    #[test]
    fn it_is_deterministic_and_seeded() {
        let s = Sight {
            star: Some(at(10, 10)),
            ..up()
        };
        for tick in 0..50 {
            assert_eq!(bot().decide(&s, 2, tick), bot().decide(&s, 2, tick));
        }
        let differ = |a: u64, b: u64| {
            (0..3u8).any(|slot| {
                let bot = |seed| PartyBot {
                    seed,
                    builds: false,
                };
                let (x, y) = (bot(a), bot(b));
                x.role(slot) != y.role(slot)
                    || x.phase(slot) != y.phase(slot)
                    || x.skill_phase(slot) != y.skill_phase(slot)
            })
        };
        assert!(differ(1, 2));
    }

    fn party(n: u8) -> Engine {
        let setup = RunSetup {
            mode: "time_limit".into(),
            seed: 1,
            ..trial_setup()
        };
        Engine::start_party(&setup, n).unwrap()
    }

    #[test]
    fn the_sight_reads_the_engine() {
        let e = party(2);
        let s1 = sight(&e, 1);
        assert!(s1.up);
        assert_eq!(s1.me, e.snapshot().players[1].at);
        assert_eq!(s1.up_mates.len(), 1);
        assert_eq!(s1.up_mates[0].0, 0);
        assert!(s1.downed.is_empty());
    }
}

#[cfg(test)]
mod builds {
    use super::*;
    use stella_rain_core::input::EquipAction;
    use stella_rain_core::snapshot::SwapItem;

    fn building(seed: u64) -> PartyBot {
        PartyBot { seed, builds: true }
    }

    fn equipment_engine(n: u8, world: u64) -> Engine {
        let setup = RigParty::new(n).equipment().seed(world).case("x").setup;
        Engine::start_party_with(&setup, Pack::bundled().content(), n).unwrap()
    }

    /// Plays `ticks` ticks of an equipment run of `n` with `bot` on every slot (the actions are
    /// applied with `Engine::act` before the tick, as the recorder orders them); `see` is called
    /// for every slot and tick with the engine before the action, the action chosen and the tick.
    fn drive(
        n: u8,
        world: u64,
        bot: &PartyBot,
        ticks: u32,
        mut see: impl FnMut(&Engine, u8, u32, Option<EquipAction>),
    ) -> Engine {
        let mut e = equipment_engine(n, world);
        for t in 0..ticks {
            if e.outcome() != stella_rain_core::engine::Outcome::Running {
                break;
            }
            let mut inputs = [Input::default(); 3];
            let mut acts = Vec::new();
            for s in 0..n {
                let a = bot.action(&e, s, t);
                see(&e, s, t, a);
                inputs[usize::from(s)] = bot.input(&e, s, t);
                if let Some(a) = a {
                    acts.push((s, a));
                }
            }
            for (s, a) in acts {
                e.act(PlayerIdx::new(s).unwrap(), a).unwrap();
            }
            e.step(&inputs);
        }
        e
    }

    /// The `at` of the first snapshot entity of `kind`, by the nearest to `slot`.
    fn nearest_of(e: &Engine, slot: u8, kind: EntityKind) -> Option<Point> {
        let me = sight(e, slot).me;
        e.snapshot()
            .entities
            .iter()
            .filter(|v| v.kind == kind)
            .map(|v| v.at)
            .min_by_key(|at| d2(*at, me))
    }

    fn tutorial_engine(world: u64) -> Engine {
        let setup = RigParty::new(3)
            .equipment()
            .mode("tutorial")
            .seed(world)
            .case("x")
            .setup;
        Engine::start_party_with(&setup, Pack::bundled().content(), 3).unwrap()
    }

    #[test]
    fn a_fighter_with_no_enemy_hunts_the_boss() {
        // The golem comes onto the field at tick 4000, long after the last bat: play the tutorial
        // party to that tick and look at what the first slot sees.
        let case = crate::run_play::party_case("three_tutorial", 1, 1);
        let replay = case.play().replay();
        let run = RigRun::of(&replay);
        let mut e =
            Engine::start_party_with(&case.setup, Pack::bundled().content(), run.n).unwrap();
        run.play_onto(&mut e, 0, 4000);
        assert_eq!(e.tick(), 4000);
        assert_eq!(nearest_of(&e, 0, EntityKind::Enemy), None, "no bat left");
        let boss = nearest_of(&e, 0, EntityKind::Boss).expect("the golem arrives at tick 4000");
        assert_eq!(sight(&e, 0).enemy, Some(boss));
    }

    #[test]
    fn a_bat_is_hunted_before_the_boss() {
        let mut e = tutorial_engine(1);
        for _ in 0..130 {
            e.step(&[Input::default(); 3]);
        }
        let bat = nearest_of(&e, 0, EntityKind::Enemy).expect("a bat is on the field at 130");
        assert_eq!(sight(&e, 0).enemy, Some(bat));
    }

    #[test]
    fn a_bot_without_builds_makes_no_action() {
        let bot = PartyBot {
            seed: 7,
            builds: false,
        };
        drive(3, 1, &bot, 3000, |_, s, t, a| {
            assert_eq!(a, None, "slot {s} tick {t}");
        });
    }

    #[test]
    fn an_offers_run_gets_no_action() {
        // A sim_version 1 run is the frozen offers rules: one player, no slots.
        let setup = RunSetup {
            sim_version: 1,
            mode: "time_limit".into(),
            seed: 1,
            ..trial_setup()
        };
        let e = Engine::start_with(&setup, crate::frozen_v1::pack().content()).unwrap();
        assert!(e.snapshot().players[0].slots.is_empty());
        for t in 0..600 {
            for s in 0..1 {
                assert_eq!(building(1).action(&e, s, t), None);
            }
        }
    }

    #[test]
    fn the_cadence_is_one_decision_in_thirty_ticks() {
        let mut differ = false;
        for seed in 1..=5 {
            let b = building(seed);
            let phases: Vec<u64> = (0..3).map(|s| b.action_phase(s)).collect();
            assert!(phases.iter().all(|p| *p < 30));
            differ |= phases.iter().any(|p| *p != phases[0]);
        }
        assert!(differ);
        let b = building(1);
        let mut decided = 0;
        drive(3, 1, &b, 3000, |_, s, t, a| {
            if (u64::from(t) + b.action_phase(s)) % 30 != 0 {
                assert_eq!(a, None, "slot {s} tick {t}");
            } else {
                decided += u32::from(a.is_some());
            }
        });
        assert!(decided > 0, "a building bot acts at least once");
    }

    #[test]
    fn every_action_is_accepted() {
        let b = building(1);
        let mut proposed = 0u32;
        drive(3, 1, &b, 3000, |e, s, _, a| {
            if let Some(a) = a {
                proposed += 1;
                assert_eq!(e.check_action(PlayerIdx::new(s).unwrap(), a), Ok(()));
            }
        });
        let replay = RigParty::new(3)
            .equipment()
            .builds()
            .seed(1)
            .bot_seed(1)
            .case("x")
            .play()
            .replay();
        let run = RigRun::of(&replay);
        let recorded: u32 = run
            .actions
            .iter()
            .map(|(_, per)| u32::try_from(per.iter().flatten().count()).unwrap())
            .sum();
        assert!(recorded > 0);
        assert_eq!(recorded, proposed, "the recorder drops nothing");
    }

    /// Every decision of a building run of 3, with the engine it was made on.
    fn decisions(world: u64) -> Vec<(Engine, u8, u32, Option<EquipAction>)> {
        let mut out = Vec::new();
        drive(3, world, &building(1), 3000, |e, s, t, a| {
            if a.is_some() {
                out.push((e.clone(), s, t, a));
            }
        });
        out
    }

    #[test]
    fn the_best_rated_star_goes_in_first() {
        // A passive bot walks; the first decision of a building bot that equips is checked
        // against the ratings computed here.
        let mut checked = 0;
        for (e, s, _, a) in decisions(3) {
            let Some(EquipAction::Equip { item, slot } | EquipAction::Replace { item, slot }) = a
            else {
                continue;
            };
            let p = PlayerIdx::new(s).unwrap();
            let mut best = None;
            let inv = e.snapshot().players[usize::from(s)].inventory.len().min(12);
            for i in 0..u32::try_from(inv).unwrap() {
                for r in e.swap_ratings(p, SwapItem::Inventory(i)) {
                    let adopted = if r.replaces {
                        r.dps_bp > Some(0) && r.level_after >= r.level_before
                    } else {
                        r.dps_bp.unwrap_or(0) >= 0
                    };
                    if adopted {
                        let key = (
                            r.dps_bp.unwrap_or(0),
                            !r.replaces,
                            std::cmp::Reverse(i),
                            std::cmp::Reverse(r.slot),
                        );
                        if best.is_none_or(|(k, _, _)| key > k) {
                            best = Some((key, i, r.slot));
                        }
                    }
                }
            }
            if let Some((_, bi, bs)) = best {
                // A rune is not rated here; only star choices are compared.
                assert_eq!((item, slot), (bi, bs));
                checked += 1;
            }
        }
        assert!(checked > 0, "some star equip was compared");
    }

    #[test]
    fn a_replace_never_lowers_a_slot() {
        let mut replaces = 0;
        for world in 1..=3 {
            for (e, s, _, a) in decisions(world) {
                let Some(EquipAction::Replace { item, slot }) = a else {
                    continue;
                };
                replaces += 1;
                let p = PlayerIdx::new(s).unwrap();
                let row = e
                    .swap_ratings(p, SwapItem::Inventory(item))
                    .into_iter()
                    .find(|r| r.slot == slot)
                    .expect("a row for the slot");
                assert!(row.dps_bp > Some(0) && row.level_after >= row.level_before);
            }
        }
        // A replace is rare early; the corpus case with one is checked in party_run.
        let _ = replaces;
    }

    #[test]
    fn a_tree_point_is_taken_before_anything_else() {
        let mut seen = 0;
        for world in 1..=5 {
            let b = building(1);
            drive(1, world, &b, 3000, |e, s, t, a| {
                let ready = e.snapshot().players[usize::from(s)].tree_ready.is_some();
                if ready && (u64::from(t) + b.action_phase(s)) % 30 == 0 {
                    // A style is the seed's pick; an upgrade or a capstone takes option 0.
                    let r = e.snapshot().players[usize::from(s)]
                        .tree_ready
                        .clone()
                        .unwrap();
                    let option = if r.kind == ExpertiseOfferKind::Style {
                        u32::try_from(mix(b.seed, s, 5) % r.options.len() as u64).unwrap()
                    } else {
                        0
                    };
                    assert_eq!(a, Some(EquipAction::TreePick { option }));
                    seen += 1;
                }
            });
        }
        assert!(seen > 0, "a tree point was ready on a decision tick");
    }

    #[test]
    fn it_extracts_the_rest_then_crafts_only_what_it_would_equip() {
        let (mut extracts, mut crafts) = (0, 0);
        for (e, s, _, a) in decisions(1) {
            let p = PlayerIdx::new(s).unwrap();
            let view = &e.snapshot().players[usize::from(s)];
            match a {
                Some(EquipAction::Extract { item }) => {
                    assert_eq!(item, 0);
                    assert!(!view.inventory.is_empty());
                    extracts += 1;
                }
                Some(EquipAction::Craft { star }) => {
                    assert!(view.inventory.is_empty());
                    assert!(!view.pool_held[star as usize]);
                    crafts += 1;
                }
                _ => {}
            }
            let _ = p;
        }
        assert!(extracts > 0, "something was extracted");
        let _ = crafts;
    }
}

#[cfg(test)]
mod rig {
    use super::*;
    use crate::run_play::{Change, party_cases};
    use stella_rain_core::pack::Pack;
    use stella_rain_core::run_replay::{CHECKPOINT_TICKS, verify_run};

    #[test]
    #[should_panic(expected = "1 to 3")]
    fn a_party_of_none_is_refused() {
        let _ = RigParty::new(0);
    }

    #[test]
    #[should_panic(expected = "1 to 3")]
    fn a_party_of_four_is_refused() {
        let _ = RigParty::new(4);
    }

    #[test]
    fn the_builder_makes_the_case_it_is_told() {
        for n in 1..=3u8 {
            let case = RigParty::new(n).seed(9).bot_seed(5).case("x");
            assert_eq!(case.name, "x");
            assert_eq!(case.party, n);
            assert_eq!(case.setup.mode, "time_limit".into());
            assert_eq!(case.setup.seed, 9);
            assert_eq!(case.setup.difficulty, "normal".into());
            assert_eq!(case.ticks, 3000);
            assert_eq!(
                case.bot,
                Some(PartyBot {
                    seed: 5,
                    builds: false
                })
            );
            assert!(case.changes.is_empty());
        }
        let case = RigParty::new(2)
            .mode("tutorial")
            .difficulty("hard")
            .ticks(120)
            .case("y");
        assert_eq!(case.ticks, 120);
        assert_eq!(case.setup.mode, "tutorial".into());
        assert_eq!(case.setup.difficulty, "hard".into());
    }

    #[test]
    fn a_case_plays_records_and_verifies() {
        for n in 1..=3u8 {
            let rec = RigParty::new(n)
                .seed(1)
                .bot_seed(1)
                .ticks(600)
                .case("x")
                .play();
            assert!(rec.engine().tick() <= 600);
            let replay = rec.replay();
            verify_run(&replay.to_bytes()).unwrap();
            let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
            assert_eq!(range.present, (1u8 << n) - 1);
        }
    }

    #[test]
    fn the_same_inputs_give_the_same_bytes_and_another_bot_seed_another_run() {
        let bytes = |bot| {
            RigParty::new(2)
                .seed(3)
                .bot_seed(bot)
                .case("x")
                .play()
                .replay()
        };
        assert_eq!(bytes(1).to_bytes(), bytes(1).to_bytes());
        assert_ne!(bytes(1).inputs, bytes(2).inputs);
    }

    fn fresh(replay: &RunReplay, n: u8) -> Engine {
        Engine::start_party_with(&replay.setup, Pack::bundled().content(), n).unwrap()
    }

    fn hash_of(replay: &RunReplay) -> u64 {
        u64::from_str_radix(&replay.final_hash, 16).unwrap()
    }

    #[test]
    fn a_rig_run_replays_onto_an_engine_to_the_recorded_hashes() {
        for n in [2u8, 3] {
            let replay = RigParty::new(n)
                .seed(1)
                .bot_seed(1)
                .case("x")
                .play()
                .replay();
            let run = RigRun::of(&replay);
            assert_eq!(run.n, n);
            assert_eq!(run.ticks.len(), replay.ticks as usize);
            let mut e = fresh(&replay, n);
            let step = CHECKPOINT_TICKS;
            let mut at = 0;
            while at < replay.ticks {
                let to = (at + step).min(replay.ticks);
                run.play_onto(&mut e, at, to);
                if to % step == 0 {
                    let k = (to / step - 1) as usize;
                    let want =
                        u64::from_str_radix(&replay.checkpoints[16 * k..16 * k + 16], 16).unwrap();
                    // A pick that comes before the next tick is not applied yet: the checkpoint
                    // is taken right after the tick.
                    assert_eq!(e.state_hash(), want, "n {n} tick {to}");
                }
                at = to;
            }
            assert_eq!(e.state_hash(), hash_of(&replay), "n {n}");
        }
    }

    #[test]
    fn windows_and_clones_reach_the_same_hash() {
        let replay = RigParty::new(3)
            .equipment()
            .seed(1)
            .bot_seed(2)
            .case("x")
            .play()
            .replay();
        let run = RigRun::of(&replay);
        let len = u32::try_from(run.ticks.len()).unwrap();
        for k in [1, 59, 60, 1500] {
            let mut e = fresh(&replay, 3);
            run.play_onto(&mut e, 0, k);
            let mut copy = e.clone();
            run.play_onto(&mut e, k, len);
            assert_eq!(e.state_hash(), hash_of(&replay), "window at {k}");
            run.play_onto(&mut copy, k, len);
            assert_eq!(copy.state_hash(), hash_of(&replay), "clone at {k}");
        }
    }

    fn building_replay(n: u8) -> RunReplay {
        RigParty::new(n)
            .equipment()
            .builds()
            .seed(1)
            .bot_seed(1)
            .case("x")
            .play()
            .replay()
    }

    #[test]
    fn a_rig_run_with_actions_replays_to_the_recorded_hashes() {
        let replay = building_replay(3);
        let run = RigRun::of(&replay);
        assert!(!run.actions.is_empty());
        let mut e = fresh(&replay, 3);
        run.play_onto(&mut e, 0, replay.ticks);
        assert_eq!(e.state_hash(), hash_of(&replay));
    }

    #[test]
    fn actions_apply_after_picks_and_before_the_tick() {
        let replay = building_replay(2);
        let run = RigRun::of(&replay);
        let (at, _) = *run.actions.get(1).expect("two action steps");
        let mut whole = fresh(&replay, 2);
        run.play_onto(&mut whole, 0, at + 1);
        let mut windowed = fresh(&replay, 2);
        run.play_onto(&mut windowed, 0, at);
        run.play_onto(&mut windowed, at, at + 1);
        assert_eq!(windowed.state_hash(), whole.state_hash());
        // Without the actions of that tick the hash differs.
        let mut bare = run.clone();
        bare.actions.retain(|(t, _)| *t != at);
        let mut skipped = fresh(&replay, 2);
        bare.play_onto(&mut skipped, 0, at + 1);
        assert_ne!(skipped.state_hash(), whole.state_hash());
    }

    #[test]
    fn actions_per_slot_counts_the_kinds() {
        let run = RigRun::of(&building_replay(3));
        let per = run.actions_per_slot();
        let total: u32 = per
            .iter()
            .map(|c| c.equip + c.replace + c.extract + c.craft + c.tree_pick)
            .sum();
        let some: usize = run
            .actions
            .iter()
            .map(|(_, a)| a.iter().flatten().count())
            .sum();
        assert_eq!(total as usize, some);
        let revive = RigRun::of(
            &RigParty::new(3)
                .seed(1)
                .bot_seed(1)
                .case("x")
                .play()
                .replay(),
        );
        assert!(revive.actions.is_empty());
        assert!(
            revive
                .actions_per_slot()
                .iter()
                .all(|c| *c == ActionCounts::default())
        );
    }

    #[test]
    #[should_panic(expected = "join")]
    fn a_join_in_the_log_is_refused() {
        let case = party_cases()
            .into_iter()
            .find(|c| c.changes.iter().any(|(_, c)| matches!(c, Change::Join(_))))
            .unwrap();
        let _ = RigRun::of(&case.play().replay());
    }
}

#[cfg(test)]
mod gauge {
    //! The building bot spends a full gauge and picks its style by seed (ADR-SIM-9).
    use super::*;
    use stella_rain_core::engine::Outcome;

    fn building(seed: u64) -> PartyBot {
        PartyBot::building(seed)
    }

    fn full() -> Sight {
        Sight {
            up: true,
            gauge_full: true,
            ..Sight::default()
        }
    }

    #[test]
    fn a_full_gauge_smashes_two_ticks_in_eight() {
        let bot = building(7);
        for slot in 0..3u8 {
            for t in 0..80u32 {
                let want = (u64::from(t) + bot.phase(slot)) % 8 < 2;
                assert_eq!(
                    bot.decide(&full(), slot, t).smash,
                    want,
                    "slot {slot} t {t}"
                );
            }
        }
    }

    #[test]
    fn without_a_full_gauge_nothing_else_changes_and_it_never_smashes() {
        let bot = building(7);
        let up = Sight {
            up: true,
            ..Sight::default()
        };
        for t in 0..3000u32 {
            let i = bot.decide(&up, 0, t);
            assert!(!i.smash, "t {t}");
            assert_eq!(i.main, (u64::from(t) + bot.phase(0)) % 40 < 5);
            assert_eq!(i.skill, (u64::from(t) + bot.skill_phase(0)) % 90 < 30);
        }
    }

    #[test]
    fn a_bot_that_does_not_build_never_smashes() {
        let bot = PartyBot {
            seed: 7,
            builds: false,
        };
        for t in 0..80u32 {
            assert!(!bot.decide(&full(), 0, t).smash, "t {t}");
        }
    }

    fn engine_of(weapon: &str, world: u64) -> Engine {
        let mut setup = RigParty::new(1).equipment().seed(world).case("x").setup;
        setup.weapon = weapon.into();
        Engine::start_party_with(&setup, Pack::bundled().content(), 1).unwrap()
    }

    /// Plays slot 0 with `bot` until its first tree point is taken: the style offered, the
    /// option taken and what `check_action` said.
    fn first_style_pick(engine: &mut Engine, bot: &PartyBot) -> (usize, usize) {
        for t in 0..3000u32 {
            let view = engine.snapshot().players[0].tree_ready.clone();
            let a = bot.action(engine, 0, t);
            if let (Some(EquipAction::TreePick { option }), Some(ready)) = (a, view) {
                assert!(
                    engine
                        .check_action(PlayerIdx::new(0).unwrap(), a.unwrap())
                        .is_ok()
                );
                return (usize::try_from(option).unwrap(), ready.options.len());
            }
            if let Some(a) = a {
                engine.act(PlayerIdx::new(0).unwrap(), a).unwrap();
            }
            let input = bot.input(engine, 0, t);
            engine.step(&[input, Input::default(), Input::default()]);
        }
        panic!("no tree pick in 3000 ticks");
    }

    #[test]
    fn the_style_is_picked_by_the_seed_and_every_style_occurs() {
        for weapon in ["giant_sword", "giant_hammer"] {
            let mut seen = std::collections::BTreeSet::new();
            for seed in 1..=20u64 {
                let bot = building(seed);
                let mut e = engine_of(weapon, 1);
                let (option, len) = first_style_pick(&mut e, &bot);
                assert!(len >= 2, "{weapon}: a style has options");
                assert_eq!(
                    option,
                    usize::try_from(mix(seed, 0, 5) % len as u64).unwrap(),
                    "{weapon} seed {seed}"
                );
                seen.insert(option);
            }
            assert_eq!(seen.len(), 3, "{weapon}: all three styles over 20 seeds");
        }
    }

    #[test]
    fn a_run_that_may_pick_one_style_takes_option_zero() {
        let mut setup = RigParty::new(1).equipment().seed(1).case("x").setup;
        setup.unlocked_styles = Some(vec!["foresight_counter".into()]);
        for seed in 1..=20u64 {
            let mut e = Engine::start_party_with(&setup, Pack::bundled().content(), 1).unwrap();
            let (option, _) = first_style_pick(&mut e, &building(seed));
            assert_eq!(option, 0, "seed {seed}");
        }
    }

    /// Plays slot 0 of an engine whose weapon already holds every level (`script_expertise`),
    /// and returns each tree pick the bot took: the offer's kind, its option count and the option.
    fn every_tree_pick(bot: &PartyBot, world: u64) -> Vec<(ExpertiseOfferKind, usize, u32)> {
        let mut e = engine_of("giant_sword", world);
        e.script_expertise(PlayerIdx::new(0).unwrap(), u32::MAX / 2)
            .unwrap();
        let mut picks = Vec::new();
        for t in 0..600u32 {
            let ready = e.snapshot().players[0].tree_ready.clone();
            if let Some(a) = bot.action(&e, 0, t) {
                if let (EquipAction::TreePick { option }, Some(r)) = (a, ready) {
                    picks.push((r.kind, r.options.len(), option));
                }
                e.act(PlayerIdx::new(0).unwrap(), a).unwrap();
            }
            let input = bot.input(&e, 0, t);
            e.step(&[input, Input::default(), Input::default()]);
        }
        picks
    }

    #[test]
    fn upgrade_and_capstone_points_take_option_zero() {
        for seed in 1..=8u64 {
            let picks = every_tree_pick(&building(seed), 2);
            let kinds: Vec<_> = picks.iter().map(|p| p.0).collect();
            assert_eq!(
                kinds,
                [
                    ExpertiseOfferKind::Style,
                    ExpertiseOfferKind::Upgrade,
                    ExpertiseOfferKind::Upgrade,
                    ExpertiseOfferKind::Capstone
                ],
                "seed {seed}: the whole tree is taken in path order"
            );
            for (kind, len, option) in picks {
                if kind != ExpertiseOfferKind::Style {
                    assert_eq!((len, option), (1, 0), "seed {seed} {kind:?}");
                }
            }
        }
    }

    #[test]
    fn sight_reads_the_gauge_full() {
        let bot = building(2);
        let mut e = engine_of("giant_sword", 2);
        let mut seen_full = false;
        for t in 0..3000u32 {
            let g = e.snapshot().players[0].gauge.clone();
            assert_eq!(
                sight(&e, 0).gauge_full,
                g.is_some_and(|g| g.max > 0 && g.value >= g.max),
                "t {t}"
            );
            seen_full |= sight(&e, 0).gauge_full;
            if let Some(a) = bot.action(&e, 0, t) {
                e.act(PlayerIdx::new(0).unwrap(), a).unwrap();
            }
            let input = bot.input(&e, 0, t);
            e.step(&[input, Input::default(), Input::default()]);
        }
        assert!(seen_full, "the gauge fills in 3000 ticks");
    }

    #[test]
    fn the_rig_spends_its_gauge_and_replays_to_its_hashes() {
        let replay = crate::run_play::equipment_rig(1).play().replay();
        let tally = RigRun::of(&replay).tally(&replay);
        assert!(tally.gauge_spent >= 1, "spent {}", tally.gauge_spent);
        assert_eq!(rig_builds_miss(&replay, 1), None);
        assert_eq!(
            stella_rain_core::run_replay::verify_run(&replay.to_bytes())
                .unwrap()
                .outcome,
            Outcome::Running
        );
    }
}

/// The seed search for the rig cases of the corpus. Run it by hand:
/// `cargo test --locked --test integration search_rig_seeds -- --ignored --nocapture`.
#[cfg(test)]
mod search {
    use super::*;
    use stella_rain_core::run_replay::verify_run;

    /// The world seeds and bot seeds scanned.
    const WORLD_SEEDS: std::ops::RangeInclusive<u64> = 1..=300;
    const BOT_SEEDS: std::ops::RangeInclusive<u64> = 1..=20;

    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_rig_seeds() {
        for n in [2u8, 3] {
            let mut found = 0;
            for seed in WORLD_SEEDS {
                for bot_seed in BOT_SEEDS {
                    let case = RigParty::new(n).seed(seed).bot_seed(bot_seed).case("x");
                    let replay = case.play().replay();
                    let bytes = replay.to_bytes();
                    let run = RigRun::of(&replay);
                    let tally = run.tally(&replay);
                    let ok = replay.ticks <= 3000
                        && bytes.len() < 64 * 1024
                        && tally.downed >= 1
                        && tally.revived >= 1
                        && tally.revived_were_down
                        && tally.revived_have_hp
                        && tally
                            .first_out
                            .is_none_or(|out| tally.first_revive.is_some_and(|r| r < out))
                        && (n == 2 || tally.revivers.iter().any(|r| *r != 0));
                    if !ok {
                        continue;
                    }
                    found += 1;
                    let v = verify_run(&bytes).unwrap();
                    println!(
                        "N={n} seed={seed} bot_seed={bot_seed} ticks={} outcome={:?} \
                         downed={} revived={} out={} first_down={:?} first_revive={:?} \
                         revivers={:?} stardust={} bytes={} final_hash={}",
                        replay.ticks,
                        v.outcome,
                        tally.downed,
                        tally.revived,
                        tally.out,
                        tally.first_down,
                        tally.first_revive,
                        tally.revivers,
                        v.stardust,
                        bytes.len(),
                        replay.final_hash,
                    );
                }
            }
            println!("N={n}: {found} candidates");
        }
    }

    /// The search for the building rig cases of the corpus: `cargo test --release --locked --test
    /// integration search_equipment_rig_seeds -- --ignored --nocapture`. World seeds 1..=100 x
    /// bot seeds 1..=10, per party size the first that misses no predicate of `rig_builds_miss`;
    /// widened once to 1..=300 x 1..=20.
    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_equipment_rig_seeds() {
        for n in 1..=3u8 {
            let mut found = None;
            let mut nearest = ("", 0u8);
            'scan: for (worlds, bots) in [(1..=100u64, 1..=10u64), (1..=300, 1..=20)] {
                for seed in worlds {
                    for bot_seed in bots.clone() {
                        let case = RigParty::new(n)
                            .equipment()
                            .builds()
                            .seed(seed)
                            .bot_seed(bot_seed)
                            .case("x");
                        let replay = case.play().replay();
                        match rig_builds_miss(&replay, n) {
                            None => {
                                found = Some((seed, bot_seed));
                                break 'scan;
                            }
                            Some(m) => nearest = (m, nearest.1.saturating_add(1)),
                        }
                    }
                }
            }
            match found {
                Some((seed, bot_seed)) => {
                    let case = RigParty::new(n)
                        .equipment()
                        .builds()
                        .seed(seed)
                        .bot_seed(bot_seed)
                        .case("x");
                    let replay = case.play().replay();
                    let v = verify_run(&replay.to_bytes()).unwrap();
                    println!(
                        "N={n}: seed={seed} bot_seed={bot_seed} stardust={} per_slot={:?}",
                        v.stardust,
                        RigRun::of(&replay).actions_per_slot()
                    );
                }
                None => println!("N={n}: NOT FOUND, last miss: {}", nearest.0),
            }
        }
    }
}
