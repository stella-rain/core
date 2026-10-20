//! Default damage and hit points scale by 100 (ADR-103 decision 6): where no percentage
//! applies, a run's outcome does not change. The outcomes below were recorded under the rules
//! before the scale (core v0.0.13): the corpus stages written at design values and scaled with
//! `common::scaled`, and the recorded run cases, must end the same tick, with the same kills
//! and, for a stage, the main character's hit points x100.

use crate::common::{self, SCALE};
use crate::corpus::{case_content, definitions, runs_dir};
use std::fs;
use std::path::Path;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_replay::{CommandKind, RunReplay};
use stella_rain_core::validate::parse_and_validate;

fn kills(engine: &Engine) -> usize {
    engine
        .events()
        .iter()
        .filter(|e| matches!(e, DomainEvent::Died { .. }))
        .count()
}

/// Outcome, end tick, deaths and the main character's hit points per corpus stage, at the
/// design values.
const STAGES: [(&str, Outcome, u32, usize, u32); 10] = [
    ("agents", Outcome::Running, 80, 1, 99),
    ("assist", Outcome::Running, 300, 0, 99),
    ("boss", Outcome::Cleared, 111, 1, 99),
    ("bullets", Outcome::Running, 300, 0, 99),
    ("clear", Outcome::Cleared, 35, 1, 3),
    ("evade", Outcome::Failed, 300, 0, 97),
    ("fail", Outcome::Failed, 155, 0, 0),
    ("landscape", Outcome::Failed, 300, 1, 98),
    ("melee", Outcome::Running, 300, 3, 97),
    ("timeout", Outcome::Failed, 90, 0, 3),
];

#[test]
fn a_stage_scaled_by_100_ends_as_it_did_at_design_values() {
    let cases = definitions();
    assert_eq!(cases.len(), STAGES.len());
    for (name, bytes, inputs) in cases {
        let scaled = common::scaled_bytes(&bytes, SCALE);
        let stage = parse_and_validate(&scaled).unwrap();
        let mut engine = Engine::new(&stage).unwrap();
        let mut died = 0;
        for input in &inputs {
            engine.step(*input);
            died += kills(&engine);
        }
        let hp = engine.snapshot().player.hp;
        let (_, outcome, ticks, deaths, left) = *STAGES.iter().find(|s| s.0 == name).unwrap();
        assert_eq!(
            (engine.outcome(), engine.tick(), died, hp),
            (outcome, ticks, deaths, left * SCALE as u32),
            "`{name}` scaled by 100"
        );
    }
}

/// Where a v0.0.13 run case, played on the scaled rules, first differs from what it was.
struct Divergence {
    case: &'static str,
    /// The tick of the first hit that differs, and how many events (hits, hits on the main
    /// character and deaths) came before it.
    tick: u32,
    before: usize,
    /// FNV-1a 64 of those events, one line each (`HIT tick source target damage`, `PHIT tick
    /// hit_points_left`, `DIED tick entity`), damage and hit points ÷ 100, as the old rules
    /// recorded them.
    prefix_hash: u64,
    /// The hit that differs: the main character (0) on `target`, for `old_damage` then and
    /// `new_damage` now.
    target: u32,
    old_damage: u32,
    new_damage: u32,
    /// The percentage that no longer rounds down.
    why: &'static str,
}

const DIVERGENCES: [Divergence; 6] = [
    Divergence {
        case: "elites",
        tick: 4_085,
        before: 29,
        prefix_hash: 0xbdd4_70ae_4a1d_ce6e,
        target: 5,
        old_damage: 10,
        new_damage: 1_050,
        why: "attack +5% of 10 is 10; of 1,000 it is 1,050",
    },
    Divergence {
        case: "stage_skill",
        tick: 920,
        before: 25,
        prefix_hash: 0xd99a_5e09_23a8_02bc,
        target: 4,
        old_damage: 8,
        new_damage: 880,
        why: "attack +10% and a resisted type: 10 x 110% x 80% is 8.8, cut to 8; 880 whole",
    },
    Divergence {
        case: "time_limit",
        tick: 1_426,
        before: 69,
        prefix_hash: 0xd17f_d492_7324_ed80,
        target: 35,
        old_damage: 16,
        new_damage: 1_650,
        why: "attack +10% of the explosion rune's 15 is 16.5, cut to 16; 1,650 whole",
    },
    Divergence {
        case: "time_limit_kill",
        tick: 437,
        before: 17,
        prefix_hash: 0xb44d_c212_e3b1_22fb,
        target: 2,
        old_damage: 8,
        new_damage: 880,
        why: "attack +10% and a resisted type: 8.8 cut to 8; 880 whole",
    },
    Divergence {
        case: "time_limit_overtime",
        tick: 437,
        before: 17,
        prefix_hash: 0xb44d_c212_e3b1_22fb,
        target: 2,
        old_damage: 8,
        new_damage: 880,
        why: "attack +10% and a resisted type: 8.8 cut to 8; 880 whole",
    },
    Divergence {
        case: "trial",
        tick: 898,
        before: 23,
        prefix_hash: 0x4f3f_65c0_2b72_8411,
        target: 4,
        old_damage: 8,
        new_damage: 880,
        why: "attack +10% and a resisted type: 8.8 cut to 8; 880 whole",
    },
];

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// An event of the log, as the engine reports it.
#[derive(Debug)]
enum Logged {
    Hit {
        tick: u32,
        source: u32,
        target: u32,
        damage: u32,
    },
    PlayerHit {
        tick: u32,
        hp_left: u32,
    },
    Died {
        tick: u32,
        entity: u32,
    },
}

impl Logged {
    /// The line the old rules logged, the numbers ÷ 100.
    fn line(&self, case: &str) -> String {
        let whole = |n: u32| {
            assert_eq!(n % 100, 0, "`{case}`: {self:?} is not a whole hundred");
            n / 100
        };
        match *self {
            Logged::Hit {
                tick,
                source,
                target,
                damage,
            } => format!("HIT {tick} {source} {target} {}\n", whole(damage)),
            Logged::PlayerHit { tick, hp_left } => format!("PHIT {tick} {}\n", whole(hp_left)),
            Logged::Died { tick, entity } => format!("DIED {tick} {entity}\n"),
        }
    }
}

#[test]
fn the_v0_0_13_run_cases_play_the_same_on_the_scaled_rules_until_a_percentage_applies() {
    for d in &DIVERGENCES {
        let content = case_content(&runs_dir().join(d.case)).unwrap();
        let bytes = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/x100")
                .join(format!("{}.replay", d.case)),
        )
        .unwrap();
        let replay = RunReplay::from_bytes(&bytes).unwrap();
        let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
        let commands = replay.unpack_commands().unwrap();
        let mut next = commands.iter().peekable();
        let mut log = Vec::new();
        for input in replay.unpack_inputs(u32::MAX).unwrap() {
            while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
                match c.kind {
                    CommandKind::Choose => engine.choose(c.index).unwrap(),
                    CommandKind::Spend => engine.spend(c.index).unwrap(),
                }
            }
            engine.step(input);
            let tick = engine.tick();
            for event in engine.events() {
                log.push(match *event {
                    DomainEvent::Hit {
                        source,
                        target,
                        damage,
                        ..
                    } => Logged::Hit {
                        tick,
                        source: source.0,
                        target: target.0,
                        damage,
                    },
                    DomainEvent::PlayerHit { hp_left } => Logged::PlayerHit { tick, hp_left },
                    DomainEvent::Died { entity } => Logged::Died {
                        tick,
                        entity: entity.0,
                    },
                    _ => continue,
                });
            }
            if tick >= d.tick {
                break;
            }
        }
        let before: String = log[..d.before].iter().map(|l| l.line(d.case)).collect();
        assert_eq!(fnv1a(before.as_bytes()), d.prefix_hash, "`{}`", d.case);
        let Logged::Hit {
            tick,
            source,
            target,
            damage,
        } = log[d.before]
        else {
            panic!("`{}`: {:?} is not a hit", d.case, log[d.before]);
        };
        assert_eq!(
            (tick, source, target, damage),
            (d.tick, 0, d.target, d.new_damage),
            "`{}`: {}",
            d.case,
            d.why
        );
        assert_ne!(damage, d.old_damage * 100, "`{}` differs", d.case);
    }
}
