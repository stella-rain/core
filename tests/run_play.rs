//! The scripted run of the trial and the other modes as a list of commands: `run_input`'s
//! ticks and every offer answered (`common::run::answer_all`); the setup's stat priority spends the
//! level-ups. Beside the common fixtures, not in them, so that the benches that
//! share `tests/common/` do not build it.

use crate::common;
use crate::common::run::{STEER_FROM, answer_all, trial_setup};
use crate::party_bot::{PartyBot, RigParty};
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunRecorder};
use stella_rain_core::run_setup::{OfferMode, RunSetup};
use stella_rain_core::snapshot::HeldView;

/// The commands of a run of `run_input` for up to `ticks` ticks, answering every offer as
/// `answer_all` does, between a tick and the next (and after the last).
pub fn play_run(ticks: u32) -> Vec<Command> {
    play_run_from(&trial_setup(), ticks)
}

/// `play_run` from `setup` instead of the trial's.
pub fn play_run_from(setup: &RunSetup, ticks: u32) -> Vec<Command> {
    play_run_in(setup, RunContent::bundled(), ticks)
}

/// `play_run_from` with `content` in place of the bundled files.
pub fn play_run_in(setup: &RunSetup, content: &RunContent, ticks: u32) -> Vec<Command> {
    play_run_with(setup, content, ticks, run_input)
}

/// `play_run_in` with `input` choosing each tick's buttons.
pub fn play_run_with(
    setup: &RunSetup,
    content: &RunContent,
    ticks: u32,
    input: impl Fn(&Engine, u32) -> Input,
) -> Vec<Command> {
    let mut engine = Engine::start_with(setup, content).unwrap();
    let mut out = Vec::new();
    let mut offers = 0;
    let between = |engine: &mut Engine, out: &mut Vec<Command>, offers: &mut usize| {
        answer_all(engine, offers, out);
    };
    for t in 0..ticks {
        if engine.outcome() != Outcome::Running {
            break;
        }
        between(&mut engine, &mut out, &mut offers);
        let input = input(&engine, t);
        engine.step(&[input, Default::default(), Default::default()]);
        out.push(Command::Step(input));
    }
    between(&mut engine, &mut out, &mut offers);
    out
}

/// The tick at whose end the run's first level-up comes: the one that raises the first stat.
/// Offers may open before it.
pub fn first_point_tick() -> u32 {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for command in play_run(STEER_FROM) {
        match command {
            Command::Step(input) => engine.step(&[input, Default::default(), Default::default()]),
            Command::Choose(index) => common::pick(&mut engine, index).unwrap(),
        }
        if engine.snapshot().progress.is_some_and(|p| p.level > 1) {
            return engine.tick();
        }
    }
    panic!("the script gains no level before tick {STEER_FROM}");
}

/// How near (1/256 pixel) a scripted slot walks up to the enemy it hunts: 32 pixels.
const SCRIPT_NEAR: i64 = 32 * 256;

/// The scripted input of slot `slot` (1 or 2) of a party at `tick`: distinct patterns, so a log
/// carries each slot. Like slot 0's script they walk to the nearest star (slot 1) or enemy
/// and press `main` (`tick % 40 < 5`), so that their attacks are exercised (ADR-SIM-7); a
/// slot that only walked one way would stand at the map's edge and earn nothing.
pub fn scripted_input(engine: &Engine, slot: u8, tick: u32) -> Input {
    let s = engine.snapshot();
    // The snapshot lists the present players in slot order, so a hole shifts the index.
    let place = (engine.present() & ((1 << slot) - 1)).count_ones() as usize;
    let me = s.players[place].at;
    let d2 = |at: stella_rain_core::fixed::Point| {
        let (x, y) = (i64::from(at.x.0 - me.x.0), i64::from(at.y.0 - me.y.0));
        x * x + y * y
    };
    let toward = |d: i32| match d {
        d if d > 256 => 127,
        d if d < -256 => -127,
        _ => 0,
    };
    let star = s.stars.iter().map(|st| st.at).min_by_key(|at| d2(*at));
    let enemy = s
        .entities
        .iter()
        .filter(|v| v.kind == stella_rain_core::snapshot::EntityKind::Enemy)
        .map(|v| v.at)
        .min_by_key(|at| d2(*at));
    let goal = if slot == 1 {
        star.or(enemy)
    } else {
        enemy.or(star)
    };
    let mut input = Input {
        main: tick % 40 < 5,
        ..Input::default()
    };
    if let Some(at) = goal.filter(|at| d2(*at) > SCRIPT_NEAR * SCRIPT_NEAR) {
        input.move_x = toward(at.x.0 - me.x.0);
        input.move_y = toward(at.y.0 - me.y.0);
    }
    if slot == 1 {
        input.skill = tick % 90 < 30;
    } else {
        input.evade = tick % 120 < 10;
    }
    input
}

/// The inputs of a party at `tick`: slot 0 played by the zone bot, the others scripted; with a
/// `bot`, every present slot is played by it.
pub fn party_inputs_with(rec: &RunRecorder, tick: u32, bot: Option<&PartyBot>) -> [Input; 3] {
    let mut inputs = [Input::default(); 3];
    let present = rec.engine().present();
    for s in (0..3u8).filter(|s| present >> s & 1 == 1) {
        inputs[usize::from(s)] = match (bot, s) {
            (Some(bot), _) => bot.input(rec.engine(), s, tick),
            (None, 0) => run_input(rec.engine(), tick),
            (None, _) => scripted_input(rec.engine(), s, tick),
        };
    }
    inputs
}

/// Answers every open offer, each by the slot that holds it, one pick at a time: option 0, or
/// with a `bot` the option it picks (option 0 if it gives none).
pub fn answer_party_offers_with(rec: &mut RunRecorder, bot: Option<&PartyBot>) {
    while rec.engine().offer_open() {
        let present = rec.engine().present();
        let holder = (0..3u8)
            .filter(|s| present >> s & 1 == 1)
            .find(|i| {
                rec.engine()
                    .check_pick(PlayerIdx::new(*i).unwrap(), 0)
                    .is_ok()
            })
            .expect("an open offer belongs to a slot");
        let index = bot
            .and_then(|bot| bot.pick(rec.engine(), holder, rec.engine().tick()))
            .unwrap_or(0);
        rec.choose(PlayerIdx::new(holder).unwrap(), index).unwrap();
    }
}

/// A change of the party at a tick.
#[derive(Clone, Copy, Debug)]
pub enum Change {
    Join(u8),
    Leave(u8),
}

/// A party of `n` played from `setup` for up to `ticks` ticks, every offer answered by its slot
/// (one pick at a time between ticks); each of `changes` (world tick, change) is applied before
/// that tick's step. In a timed run no offer waits for a pick: slot 0 answers an offer (option 0)
/// `answer_after` ticks after it opened, every second offer of its own, in the input of a tick;
/// the other slots never answer, and an offer nobody answers is closed by the timer. `None`
/// answers nothing. With a `bot` it plays every present slot and picks every offer.
pub fn play_party_with(
    setup: &RunSetup,
    n: u8,
    ticks: u32,
    changes: &[(u32, Change)],
    answer_after: Option<u32>,
    bot: Option<&PartyBot>,
    script: Option<&EquipScript>,
) -> RunRecorder {
    let timed = setup.offer_mode == OfferMode::Timed;
    assert!(bot.is_none() || !timed, "a timed bot case is not built");
    let mut rec = RunRecorder::start_party(setup, n).unwrap();
    let slot0 = PlayerIdx::SOLO;
    // The world tick slot 0's current offer opened on, and the offers it has met.
    let (mut open_since, mut met): (Option<u32>, u32) = (None, 0);
    for t in 0..ticks {
        for (_, change) in changes.iter().filter(|(at, _)| *at == t) {
            match *change {
                Change::Join(s) => rec.join(PlayerIdx::new(s).unwrap()).unwrap(),
                Change::Leave(s) => rec.leave(PlayerIdx::new(s).unwrap()).unwrap(),
            }
        }
        if !timed {
            answer_party_offers_with(&mut rec, bot);
        }
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        let mut inputs = party_inputs_with(&rec, t, bot);
        inputs[0].action = script.and_then(|script| script.action(rec.engine(), t));
        if timed {
            if rec.engine().check_pick(slot0, 0).is_ok() {
                let since = *open_since.get_or_insert_with(|| {
                    met += 1;
                    rec.engine().tick()
                });
                let waited = rec.engine().tick() - since;
                if met % 2 == 1 && answer_after.is_some_and(|n| waited >= n) {
                    inputs[0].pick = Some(0);
                    open_since = None;
                }
            } else {
                open_since = None;
            }
        }
        rec.step(&inputs).unwrap();
        // An offer taken (by a pick or by the timer) ends the wait: the next one counts afresh.
        if timed
            && rec
                .engine()
                .events()
                .iter()
                .any(|ev| matches!(ev, stella_rain_core::event::DomainEvent::RewardTaken { .. }))
        {
            open_since = None;
        }
    }
    if !timed {
        answer_party_offers_with(&mut rec, bot);
    }
    rec
}

/// A party case of the corpus (`tests/corpus/current/parties/<name>/`): `party` players on
/// `setup`, played for up to `ticks` ticks by `play_party`.
pub struct PartyCase {
    pub name: &'static str,
    pub setup: RunSetup,
    pub party: u8,
    pub ticks: u32,
    /// Joins and leaves, by world tick.
    pub changes: Vec<(u32, Change)>,
    /// A timed case: the ticks slot 0 waits after an offer opens before it answers it (every
    /// second one); `None` answers none and leaves them to the timer.
    pub answer_after: Option<u32>,
    /// A bot that plays every slot and picks every offer; `None` is the zone bot and the
    /// scripted slots, option 0 for every offer.
    pub bot: Option<PartyBot>,
    /// Equipment actions slot 0 makes between ticks (ADR-SIM-7); `None` makes none.
    pub script: Option<EquipScript>,
}

/// When slot 0 equips in a party case, and what: at `rune_at` the first inventory rune that goes
/// into a rune slot, at `star_at` the first star that goes into a category slot, and at `swap_at`
/// a replace of a star by one the engine accepts (a same-option star of another grade, when the
/// inventory holds one) or, failing that, the next star into a free category slot. Each is the
/// first accepted by `Engine::check_action`, items and slots in order.
#[derive(Clone, Copy, Debug)]
pub struct EquipScript {
    pub rune_at: u32,
    pub star_at: u32,
    pub swap_at: u32,
    /// The replace at `swap_at` is by a star whose id (without its grade) differs from the slot's
    /// star, so it shares no option (the contents' ids name the option): a slot's expertise is
    /// halved. Without it the first accepted replace is made, a switch when there is one.
    pub no_share: bool,
    /// At this tick, and the two after it, slot 0 extracts inventory item 0 if the engine
    /// accepts it (ADR-SIM-7); `None` extracts nothing.
    pub extract_at: Option<u32>,
    /// At this tick slot 0 crafts the first star of the pool the engine accepts; `None` crafts
    /// nothing.
    pub craft_at: Option<u32>,
}

/// A star id without its grade: `max_hp_common` is `max_hp`.
fn stem(id: &str) -> &str {
    [
        "_supernova",
        "_legendary",
        "_uncommon",
        "_common",
        "_epic",
        "_rare",
    ]
    .iter()
    .find_map(|g| id.strip_suffix(g))
    .unwrap_or(id)
}

impl EquipScript {
    /// The action slot 0 makes before `tick`, if the script has one there.
    pub fn action(&self, engine: &Engine, tick: u32) -> Option<EquipAction> {
        let inventory = engine.snapshot().players[0].inventory.clone();
        let find = |make: fn(u32, u32) -> EquipAction, rune: bool, slots: std::ops::Range<u32>| {
            inventory
                .iter()
                .enumerate()
                .filter(|(_, h)| matches!(h, HeldView::Rune { .. }) == rune)
                .flat_map(|(item, _)| {
                    let item = u32::try_from(item).unwrap();
                    slots.clone().map(move |slot| make(item, slot))
                })
                .find(|a| engine.check_action(PlayerIdx::SOLO, *a).is_ok())
        };
        let equip = |item, slot| EquipAction::Equip { item, slot };
        let replace = |item, slot| EquipAction::Replace { item, slot };
        let slots = engine.snapshot().players[0].slots.clone();
        let other = |item: usize, slot: usize| match (&inventory[item], &slots[slot].held) {
            (HeldView::Star { id: a, .. }, Some(HeldView::Star { id: b, .. })) => {
                stem(a.as_str()) != stem(b.as_str())
            }
            _ => false,
        };
        let replace_other = || {
            (0..inventory.len())
                .flat_map(|item| (0..9usize).map(move |slot| (item, slot)))
                .filter(|&(item, slot)| other(item, slot))
                .map(|(item, slot)| {
                    replace(u32::try_from(item).unwrap(), u32::try_from(slot).unwrap())
                })
                .find(|a| engine.check_action(PlayerIdx::SOLO, *a).is_ok())
        };
        match tick {
            t if t == self.rune_at => find(equip, true, 9..12),
            t if t == self.star_at => find(equip, false, 0..9),
            t if t == self.swap_at && self.no_share => replace_other(),
            t if t == self.swap_at => {
                find(replace, false, 0..9).or_else(|| find(equip, false, 0..9))
            }
            t if self
                .extract_at
                .is_some_and(|x| t.checked_sub(x).is_some_and(|d| d < 3)) =>
            {
                let extract = EquipAction::Extract { item: 0 };
                Some(extract).filter(|a| engine.check_action(PlayerIdx::SOLO, *a).is_ok())
            }
            t if self.craft_at == Some(t) => (0..256u32)
                .map(|star| EquipAction::Craft { star })
                .find(|a| engine.check_action(PlayerIdx::SOLO, *a).is_ok()),
            _ => None,
        }
    }
}

impl PartyCase {
    /// The recording of the case.
    pub fn play(&self) -> RunRecorder {
        play_party_with(
            &self.setup,
            self.party,
            self.ticks,
            &self.changes,
            self.answer_after,
            self.bot.as_ref(),
            self.script.as_ref(),
        )
    }
}

/// An equipment case: the scripted tutorial party of `party` in equipment mode.
fn equipment(name: &'static str, party: u8) -> PartyCase {
    PartyCase {
        name,
        setup: RunSetup {
            mode: "tutorial".into(),
            seed: 2,
            build_mode: stella_rain_core::run_setup::BuildMode::Equipment,
            ..trial_setup()
        },
        party,
        ticks: 3000,
        changes: vec![],
        answer_after: None,
        bot: None,
        script: None,
    }
}

/// The world seed of `equipment_equip`.
const EQUIP_SEED: u64 = 2;

/// A solo equipment case whose slot 0 follows an `EquipScript`.
pub fn equipment_equip(name: &'static str, seed: u64) -> PartyCase {
    PartyCase {
        setup: RunSetup {
            seed,
            ..equipment(name, 1).setup
        },
        script: Some(EquipScript {
            rune_at: 1000,
            star_at: 1500,
            swap_at: 2500,
            no_share: false,
            extract_at: None,
            craft_at: None,
        }),
        ..equipment(name, 1)
    }
}

/// The world seed of `equipment_levels` (`search_equipment_levels_seeds` in `equipment_slots.rs`).
pub const EQUIP_LEVELS_SEED: u64 = 1;

/// A solo equipment case where slot 0 equips a star early and replaces it late by a star that
/// shares no option, after the star's slot has risen (the expertise of ADR-SIM-3, ADR-SIM-7).
pub fn equipment_levels(name: &'static str, seed: u64) -> PartyCase {
    PartyCase {
        setup: RunSetup {
            mode: "time_limit".into(),
            seed,
            ..equipment(name, 1).setup
        },
        script: Some(EquipScript {
            rune_at: 400,
            star_at: 700,
            swap_at: 2700,
            no_share: true,
            extract_at: None,
            craft_at: None,
        }),
        ..equipment(name, 1)
    }
}

/// The world seed of `equipment_craft` (`search_equipment_craft_seeds` in `equipment_slots.rs`).
pub const EQUIP_CRAFT_SEED: u64 = 2;

/// A solo equipment case where slot 0 equips a rune and a star, extracts three inventory items
/// and crafts a star of the pool with the Starlight they gave (ADR-SIM-7).
pub fn equipment_craft(name: &'static str, seed: u64) -> PartyCase {
    PartyCase {
        setup: RunSetup {
            seed,
            ..equipment(name, 1).setup
        },
        script: Some(EquipScript {
            rune_at: 1000,
            star_at: 1500,
            swap_at: u32::MAX,
            no_share: false,
            extract_at: Some(2000),
            craft_at: Some(2100),
        }),
        ..equipment(name, 1)
    }
}

/// The party cases. Slots 1 and 2 get an offer of their own only in some modes and seeds (the
/// seeds are found where, in the middle of the run, each answers an offer of its own): the
/// time_limit mode for two and for three. In the tutorial both slots answer, but only as the run
/// is cleared; in time_limit slot 1 answers in the middle of the run and slot 2 is not offered one
/// (no seed of 1 to 300 gives both).
pub fn party_cases() -> Vec<PartyCase> {
    let at = |mode: &str, seed: u64| RunSetup {
        mode: mode.into(),
        seed,
        ..trial_setup()
    };
    let timed = |seed: u64| RunSetup {
        offer_mode: OfferMode::Timed,
        ..at("time_limit", seed)
    };
    vec![
        PartyCase {
            name: "two_time_limit",
            setup: at("time_limit", 40),
            party: 2,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
            bot: None,
            script: None,
        },
        PartyCase {
            name: "three_tutorial",
            setup: at("tutorial", 76),
            party: 3,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
            bot: None,
            script: None,
        },
        PartyCase {
            name: "three_time_limit",
            setup: at("time_limit", 4),
            party: 3,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
            bot: None,
            script: None,
        },
        // A solo run that takes slot 1 at tick 600; slot 1 answers an offer after it.
        PartyCase {
            name: "join_two",
            setup: at("time_limit", 1),
            party: 1,
            ticks: 3000,
            changes: vec![(600, Change::Join(1))],
            answer_after: None,
            bot: None,
            script: None,
        },
        // A party of 3 that loses slot 1 (a hole, 0b101) and slot 2 (alone), then takes slot 2
        // back (0b101); slot 2 answers an offer inside the first hole window.
        PartyCase {
            name: "leave_hole",
            setup: at("time_limit", 2),
            party: 3,
            ticks: 3000,
            changes: vec![
                (900, Change::Leave(1)),
                (1500, Change::Leave(2)),
                (2100, Change::Join(2)),
            ],
            answer_after: None,
            bot: None,
            script: None,
        },
        // A timed party of one: the world keeps running with an offer open; slot 0 answers every
        // second offer 100 ticks after it opened and the timer closes the rest.
        PartyCase {
            name: "timed_one",
            setup: timed(1),
            party: 1,
            ticks: 3000,
            changes: vec![],
            answer_after: Some(100),
            bot: None,
            script: None,
        },
        // A timed party of two: slot 0 answers as above, slot 1 never does, so the timer
        // closes every offer of slot 1.
        PartyCase {
            name: "timed_two",
            setup: timed(1),
            party: 2,
            ticks: 3000,
            changes: vec![],
            answer_after: Some(100),
            bot: None,
            script: None,
        },
        // The rig cases: every slot played by a `PartyBot`, a player goes down and is revived by
        // a teammate holding `interact`. The seeds are the first of a search (`search_rig_seeds`
        // in `party_bot.rs`): world seeds 1..=300 x bot seeds 1..=20, `time_limit`, normal, 3000
        // ticks, in that scan order; a run of at most 3000 ticks and under 64 KiB with a down and a
        // revive of a downed player, no out before the first revive, and the picks of slots 1
        // and 2 adding up to at least two. For three, also a revive made by a slot other than 0.
        // The equipment cases: the scripted tutorial parties of 1 and of 2 in equipment mode, 3000
        // ticks. Every member ends holding a star and a rune; the seeds are the first of a search
        // (`search_equipment_seeds` in `equipment_drops.rs`): world seeds 1..=60, one scan per
        // party size, the first seed where every inventory holds both. The tutorial's boss is
        // not killed by the scripted slots in 3000 ticks, so the boss drop is played in
        // `equipment_drops.rs`.
        equipment("equipment_solo", 1),
        equipment("equipment_two", 2),
        // An equip and a replace in a replay (ADR-SIM-7): slot 0 equips a rune, a star and then
        // swaps or adds one, between ticks 1000 and 2500. The seed is the first of a search
        // (`search_equipment_equip_seeds` in `equipment_slots.rs`).
        equipment_equip("equipment_equip", EQUIP_SEED),
        // A star equipped early rises on the kills and is replaced late by a star sharing no
        // option: the slot keeps half its level (`search_equipment_levels_seeds`).
        equipment_levels("equipment_levels", EQUIP_LEVELS_SEED),
        // Extracts and a craft in a replay (ADR-SIM-7): the seed is the first of a search
        // (`search_equipment_craft_seeds` in `equipment_slots.rs`).
        equipment_craft("equipment_craft", EQUIP_CRAFT_SEED),
        RigParty::new(2).seed(1).bot_seed(3).case("two_revive"),
        RigParty::new(3).seed(1).bot_seed(1).case("three_revive"),
    ]
}
