//! A seeded party bot and a test-only party builder over `run_play::PartyCase` (ADR-REPL-5,
//! ADR-SIM-7, ADR-SIM-8). Beside `run_play.rs`, not in `tests/common/`, which the benches share.
//! Integer arithmetic only; every choice is a pure function of the seed, the engine, the slot
//! and the tick, so a clone of the engine can be driven again from any tick.

use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::Point;
use stella_rain_core::input::Input;

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

/// What `slot` of `engine` sees.
pub fn sight(_engine: &Engine, _slot: u8) -> Sight {
    Sight::default()
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

    /// The buttons of a slot that sees `s` at `tick`.
    pub fn decide(&self, _s: &Sight, _slot: u8, _tick: u32) -> Input {
        Input::default()
    }

    /// The buttons of `slot` at `tick`.
    pub fn input(&self, engine: &Engine, slot: u8, tick: u32) -> Input {
        self.decide(&sight(engine, slot), slot, tick)
    }

    /// The option `slot` takes from its open offer; `None` with no offer.
    pub fn pick(&self, _engine: &Engine, _slot: u8, _tick: u32) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod decisions {
    use super::*;
    use crate::common::run::trial_setup;
    use crate::run_play::scripted_input;
    use crate::zone_bot::run_input;
    use stella_rain_core::engine::ChoiceError;
    use stella_rain_core::fixed::Fx;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::run_setup::RunSetup;

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

    /// Steps `e` until the first offer of each kind has opened (answering the others with
    /// option 0 by the holder): the engine at the first star or boss offer and at the first stat
    /// or expertise offer, with the slot that holds each.
    fn first_offers(mut e: Engine, n: u8) -> (Option<(Engine, u8)>, Option<(Engine, u8)>) {
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
        assert!(got as usize <= ratings.len() - 1);
        assert!(e.check_pick(PlayerIdx::new(holder).unwrap(), got).is_ok());
        // The other slot has no offer.
        let other = 1 - holder;
        assert_eq!(b.pick(&e, other, e.tick()), None);

        if let Some((e, holder)) = stat {
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
