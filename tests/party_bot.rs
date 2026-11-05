//! A seeded party bot and a test-only party builder over `run_play::PartyCase` (ADR-REPL-5,
//! ADR-SIM-7, ADR-SIM-8). Beside `run_play.rs`, not in `tests/common/`, which the benches share.
//! Integer arithmetic only; every choice is a pure function of the seed, the engine, the slot
//! and the tick, so a clone of the engine can be driven again from any tick.

use stella_rain_core::RangeStep;
use stella_rain_core::engine::{ChoiceError, Engine};
use stella_rain_core::fixed::Point;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::run_setup::{OfferMode, RunSetup};
use stella_rain_core::snapshot::EntityKind;

use crate::common::run::trial_setup;
use crate::run_play::PartyCase;

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
}

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
    pub enemy: Option<Point>,
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
        enemy: snap
            .entities
            .iter()
            .filter(|v| v.kind == EntityKind::Enemy)
            .map(|v| v.at)
            .min_by_key(|at| d2(*at, me)),
    }
}

impl PartyBot {
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
    /// the nearest star and a fighter to the nearest enemy until within `NEAR`, then to the other.
    /// `main` and `skill` follow the slot's cadences; it never evades, which would leave the
    /// revive range.
    pub fn decide(&self, s: &Sight, slot: u8, tick: u32) -> Input {
        if !s.up {
            return Input::default();
        }
        let t = u64::from(tick);
        let mut input = Input {
            main: (t + self.phase(slot)) % 40 < 5,
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

    /// The buttons of `slot` at `tick`.
    pub fn input(&self, engine: &Engine, slot: u8, tick: u32) -> Input {
        self.decide(&sight(engine, slot), slot, tick)
    }

    /// The option `slot` takes from its open offer; `None` with no offer. A star or boss offer:
    /// the candidate with the largest `dps_bp` (none counts as 0), ties to the lowest index; a
    /// stat or expertise offer: an option chosen by the seed, the slot and the tick. Always a
    /// take, never a reroll, a skip or a banish.
    pub fn pick(&self, engine: &Engine, slot: u8, tick: u32) -> Option<u32> {
        let player = PlayerIdx::new(slot)?;
        let options = match engine.check_pick(player, u32::MAX) {
            Err(ChoiceError::OutOfRange { options, .. }) => options,
            _ => return None,
        };
        let ratings = engine.offer_ratings(player);
        if ratings.is_empty() {
            return Some(
                u32::try_from(mix(self.seed, slot, u64::from(tick)) % u64::from(options)).unwrap(),
            );
        }
        let best = ratings.iter().map(|r| r.dps_bp.unwrap_or(0)).max()?;
        let at = ratings.iter().position(|r| r.dps_bp.unwrap_or(0) == best)?;
        u32::try_from(at).ok()
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

    /// The corpus case `name`: a `stop` run, no joins or leaves, no timed answers.
    pub fn case(&self, name: &'static str) -> PartyCase {
        let setup = RunSetup {
            mode: self.mode.as_str().into(),
            seed: self.seed,
            difficulty: self.difficulty.as_str().into(),
            ..trial_setup()
        };
        assert!(
            setup.offer_mode == OfferMode::Stop,
            "a timed bot case is not built"
        );
        PartyCase {
            name,
            setup,
            party: self.n,
            ticks: self.ticks,
            changes: vec![],
            answer_after: None,
            bot: Some(PartyBot {
                seed: self.bot_seed,
            }),
        }
    }
}

/// A recorded rig run as plain data: the input of every slot at every world tick, and the
/// picks between ticks. Replayable on a bare `Engine`, or on a clone of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigRun {
    pub n: u8,
    /// The recorded input of every slot at world tick `t` (index `t`); an absent slot's is the
    /// default.
    pub ticks: Vec<[Input; 3]>,
    /// The `Picks` steps in order, each with the world tick it comes before.
    pub picks: Vec<(u32, [Option<u32>; 3])>,
}

impl RigRun {
    /// Reads `replay`'s step log; panics on a join or a leave (a rig case holds none).
    pub fn of(replay: &RunReplay) -> RigRun {
        let range = replay
            .step_range(0, replay.ticks, u32::MAX)
            .expect("a recorded step log");
        let slots: Vec<usize> = (0..3).filter(|s| range.present >> s & 1 == 1).collect();
        let spread = |dense: &[_]| {
            let mut out = [None; 3];
            for (s, v) in slots.iter().zip(dense) {
                out[*s] = *v;
            }
            out
        };
        let (mut ticks, mut picks) = (Vec::new(), Vec::new());
        for step in &range.steps {
            match step {
                RangeStep::Ticks { run, inputs } => {
                    let mut all = [Input::default(); 3];
                    for (s, input) in slots.iter().zip(inputs) {
                        all[*s] = *input;
                    }
                    ticks.extend((0..*run).map(|_| all));
                }
                RangeStep::Picks { picks: dense } => {
                    let world = u32::try_from(ticks.len()).expect("fewer ticks than u32");
                    picks.push((world, spread(dense)));
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
        }
    }

    /// Applies world ticks `from..to` to `engine`, with the picks that come before each of
    /// them, as `Engine::step` takes them; for `to == ticks.len()` also the picks after the last.
    pub fn play_onto(&self, engine: &mut Engine, from: u32, to: u32) {
        let len = u32::try_from(self.ticks.len()).expect("fewer ticks than u32");
        assert!(
            from <= to && to <= len,
            "window {from}..{to} of {len} ticks"
        );
        let picks_at = |world: u32, engine: &mut Engine| {
            for (_, picks) in self.picks.iter().filter(|(at, _)| *at == world) {
                assert!(
                    engine.offer_open(),
                    "a pick at tick {world} with no offer open"
                );
                let mut inputs = [Input::default(); 3];
                for (input, pick) in inputs.iter_mut().zip(picks) {
                    if let Some(k) = pick {
                        *input = Input::picking(*k);
                    }
                }
                engine.step(&inputs);
            }
        };
        for t in from..to {
            picks_at(t, engine);
            engine.step(&self.ticks[t as usize]);
        }
        if to == len {
            picks_at(len, engine);
        }
    }
}

#[cfg(test)]
mod decisions {
    use super::*;
    use crate::run_play::scripted_input;
    use crate::zone_bot::run_input;
    use stella_rain_core::engine::ChoiceError;
    use stella_rain_core::fixed::Fx;
    use stella_rain_core::id::PlayerIdx;

    fn at(x_px: i32, y_px: i32) -> Point {
        Point {
            x: Fx::px(x_px),
            y: Fx::px(y_px),
        }
    }

    fn bot() -> PartyBot {
        PartyBot { seed: 7 }
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
            .map(|seed| PartyBot { seed })
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
    fn buttons_follow_the_cadences_and_axes_stay_in_range() {
        for seed in 1..=3 {
            let b = PartyBot { seed };
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
                    assert!(!input.evade && !input.interact && input.pick.is_none());
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
                let (x, y) = (PartyBot { seed: a }, PartyBot { seed: b });
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

    type Held = Option<(Engine, u8)>;

    /// Steps `e` until the first offer of each kind has opened (answering the others with
    /// option 0 by the holder): the engine at the first star or boss offer and at the first stat
    /// or expertise offer, with the slot that holds each.
    fn first_offers(mut e: Engine, n: u8) -> (Held, Held) {
        let (mut star, mut stat) = (None, None);
        for t in 0..4000 {
            if e.offer_open() {
                let holder = (0..n)
                    .find(|s| e.check_pick(PlayerIdx::new(*s).unwrap(), 0).is_ok())
                    .unwrap();
                let slot = PlayerIdx::new(holder).unwrap();
                if e.offer_ratings(slot).is_empty() {
                    stat.get_or_insert_with(|| (e.clone(), holder));
                } else {
                    star.get_or_insert_with(|| (e.clone(), holder));
                }
                let mut inputs = [Input::default(); 3];
                inputs[usize::from(holder)] = Input::picking(0);
                e.step(&inputs);
                if star.is_some() && stat.is_some() {
                    break;
                }
                continue;
            }
            let mut inputs = [Input::default(); 3];
            for s in 0..n {
                inputs[usize::from(s)] = match s {
                    0 => run_input(&e, t),
                    _ => scripted_input(&e, s, t),
                };
            }
            e.step(&inputs);
        }
        (star, stat)
    }

    #[test]
    fn it_picks_the_best_rated_take_and_a_seeded_take_of_a_stat_offer() {
        let b = bot();
        let e = party(2);
        assert_eq!(b.pick(&e, 0, 0), None, "no offer at the start");
        let (star, stat) = first_offers(e, 2);
        let (e, holder) = star.expect("a star offer opens");
        let ratings = e.offer_ratings(PlayerIdx::new(holder).unwrap());
        let best = ratings.iter().map(|r| r.dps_bp.unwrap_or(0)).max().unwrap();
        let want = ratings
            .iter()
            .position(|r| r.dps_bp.unwrap_or(0) == best)
            .unwrap();
        let got = b.pick(&e, holder, e.tick()).expect("an offer is open");
        assert_eq!(got as usize, want);
        assert!((got as usize) < ratings.len());
        assert!(e.check_pick(PlayerIdx::new(holder).unwrap(), got).is_ok());
        // The other slot has no offer.
        let other = 1 - holder;
        assert_eq!(b.pick(&e, other, e.tick()), None);

        let (e, holder) = stat.expect("a stat offer opens");
        {
            let player = PlayerIdx::new(holder).unwrap();
            let Err(ChoiceError::OutOfRange { options, .. }) = e.check_pick(player, u32::MAX)
            else {
                panic!("an offer is open");
            };
            for seed in 1..=5 {
                let b = PartyBot { seed };
                let got = b.pick(&e, holder, e.tick()).unwrap();
                assert!(got < options);
                assert_eq!(Some(got), b.pick(&e, holder, e.tick()));
                assert!(e.check_pick(player, got).is_ok());
            }
        }
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
            assert_eq!(case.setup.offer_mode, OfferMode::Stop);
            assert_eq!(case.ticks, 3000);
            assert_eq!(case.bot, Some(PartyBot { seed: 5 }));
            assert!(case.changes.is_empty());
            assert_eq!(case.answer_after, None);
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
