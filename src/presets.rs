//! Attack presets (ADR-036 tier 3, ADR-055): the developer's attacks, written in the same
//! format as the attacks creators write inline, and registered as content IDs (ADR-026). A stage
//! uses one by ID with arguments, or holds an inline copy. **Taking a preset apart** copies its
//! definition inline with each argument written in as a constant; the stage then edits it
//! freely and no longer depends on the ID. The engine runs a preset by that same expansion, so a preset and
//! its inline copy run the same program and reach the same state hashes.
//!
//! The developer's weapons (ADR-084) are content, not code (WP1): `content/weapons.json`, read
//! through `weapons::Weapons`.
//!
//! Under version 0 (ADR-035) these definitions are placeholders and change in place.

use std::sync::{Arc, OnceLock};

use crate::attack::{AttackPhase, AttackRef, Direction, Emitter, Expr, PhaseKind};
use crate::attack_presets::AttackPresets;
use crate::registry::{self, Kind};
use crate::validate::Issue;
use crate::weapon::{Branches, Weapon};

/// `(id, definition)`: the definitions are JSON, the stage format's own, with `{"arg": n}` for
/// the preset's arguments. Angles are in 1/256 degree, lengths in 1/256 pixel and speeds in 1/256
/// pixel a tick. The orb patterns are one active phase of one tick that spawns its orbs;
/// the melee swings (`slam`, `slash`, `thrust`) are a wind-up, an active hit box and a recovery.
const DEFINITIONS: &[(&str, &str)] = &[
    // The five shots below were orb patterns until BR1c (DS10: nothing flies free any more);
    // their IDs and argument counts stay (ADR-CONT-2) and they are zones now: a circle where
    // the target stands, or lines from the attacker toward it. Argument 0 of the three that take one is the warning in ticks
    // (the other two warn for 15).
    // One circle where the target stands.
    (
        "aimed_single",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"zone","shape":{"type":"circle","radius":3072},"anchor":"target","wind_up_ticks":{"arg":0},"active_ticks":2}]}]"#,
    ),
    // Seven lines in a 30 degree fan at the target.
    (
        "aimed_spread",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":7,"angle":7680,"body":[
            {"type":"zone","shape":{"type":"line","length":65536,"width":3072},"wind_up_ticks":{"arg":0},"active_ticks":2,"direction":{"type":"aimed","offset":0}}]}]}]"#,
    ),
    // The elite's slam: `slam`'s blow after a 90-tick wind-up, 300 damage, the
    // attack a high elite made with one inline shot before; the wind-up is the telegraph.
    (
        "elite_slam",
        r#"[{"kind":"wind_up","ticks":90,"move_pct":50},
            {"kind":"active","ticks":4,"move_pct":50,"type":"strike","damage":300,
             "hit_box":{"type":"circle","radius":10240}},
            {"kind":"recovery","ticks":20,"move_pct":50}]"#,
    ),
    // A blow all round: a 40-pixel circle after a slow wind-up, rooted to half speed; Strike.
    (
        "slam",
        r#"[{"kind":"wind_up","ticks":12,"move_pct":50},
            {"kind":"active","ticks":4,"move_pct":50,"type":"strike",
             "hit_box":{"type":"circle","radius":10240}},
            {"kind":"recovery","ticks":20,"move_pct":50}]"#,
    ),
    // A quick sweep ahead: a 32-pixel arc, 60 degrees to each side of the facing; Slash.
    (
        "slash",
        r#"[{"kind":"wind_up","ticks":4},
            {"kind":"active","ticks":3,"type":"slash",
             "hit_box":{"type":"arc","offset":{"forward":2048},"radius":8192,"half_angle":15360}},
            {"kind":"recovery","ticks":10}]"#,
    ),
    // Forty lines, one every 3 ticks, each turned 6 degrees from the one before.
    (
        "spiral",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"repeat","count":40,"interval_ticks":3,"body":[
            {"type":"zone","shape":{"type":"line","length":65536,"width":3072},"wind_up_ticks":{"arg":0},"active_ticks":2,"direction":{"type":"sequential","step":1536}}]}]}]"#,
    ),
    // Five lines in a 20 degree fan at the target.
    (
        "spread_5",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":5,"angle":5120,"body":[
            {"type":"zone","shape":{"type":"line","length":65536,"width":3072},"wind_up_ticks":15,"active_ticks":2,"direction":{"type":"aimed","offset":0}}]}]}]"#,
    ),
    // A reach forward: a 12-pixel circle 28 pixels ahead; Pierce.
    (
        "thrust",
        r#"[{"kind":"wind_up","ticks":6},
            {"kind":"active","ticks":3,"type":"pierce",
             "hit_box":{"type":"circle","offset":{"forward":7168},"radius":3072}},
            {"kind":"recovery","ticks":12}]"#,
    ),
    // Two lines side by side, 5 degrees apart, at the target.
    (
        "twin_shot",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":2,"angle":1280,"body":[
            {"type":"zone","shape":{"type":"line","length":65536,"width":3072},"wind_up_ticks":15,"active_ticks":2,"direction":{"type":"aimed","offset":0}}]}]}]"#,
    ),
];

fn parsed() -> &'static [(&'static str, Vec<AttackPhase>)] {
    static PARSED: OnceLock<Vec<(&'static str, Vec<AttackPhase>)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        DEFINITIONS
            .iter()
            .map(|(id, json)| {
                let nodes = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("the built-in preset {id} does not parse: {e}"));
                (*id, nodes)
            })
            .collect()
    })
}

/// The ID of every preset that has a definition.
pub fn ids() -> impl Iterator<Item = &'static str> {
    DEFINITIONS.iter().map(|(id, _)| *id)
}

/// A preset's definition, with its arguments still as `arg` nodes.
pub fn definition(id: &str) -> Option<&'static [AttackPhase]> {
    parsed()
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.as_slice())
}

/// The preset `id` taken apart: its definition with each `arg` replaced by the constant in
/// `args`. `None` if there is no such preset or the number of arguments is wrong.
pub fn take_apart(presets: &AttackPresets, id: &str, args: &[i32]) -> Option<Vec<AttackPhase>> {
    let arity = registry::find(registry::ENTRIES, Kind::Preset, id)?.args;
    if usize::from(arity) != args.len() {
        return None;
    }
    Some(
        presets
            .definition(id)?
            .iter()
            .map(|p| AttackPhase {
                spawn: emitters(&p.spawn, args),
                ..p.clone()
            })
            .collect(),
    )
}

/// The phases an attack runs: a preset taken apart, or the inline phases themselves.
pub fn phases(attack: &AttackRef, presets: &AttackPresets) -> Option<Arc<Vec<AttackPhase>>> {
    match attack {
        AttackRef::Preset { id, args } => take_apart(presets, &id.0, args).map(Arc::new),
        AttackRef::Inline(phases) => Some(Arc::new(phases.clone())),
    }
}

/// Whether the phase after an attack's last `active` one is a recovery with no spawn: where a
/// held step's hold begins (ADR-084 point 6).
pub(crate) fn holds_after_active(phases: &[AttackPhase]) -> bool {
    phases
        .iter()
        .rposition(|p| p.kind == PhaseKind::Active)
        .and_then(|last| phases.get(last + 1))
        .is_some_and(|p| p.kind == PhaseKind::Recovery && p.spawn.is_empty())
}

/// The held-step checks that read a preset's phases (ADR-CONT-7 point 6, CS1b-1): what the
/// stateless validator leaves to the content that runs the weapon. A held step's attack must
/// end in a spawn-free recovery, and a hold's loop is all active phases, when either names a
/// preset. Inline attacks are the validator's. Paths are from the weapon (`$.combo.main.attack`).
pub(crate) fn hold_problems(weapon: &Weapon, presets: &AttackPresets) -> Vec<Issue> {
    fn walk(b: &Branches, at: &str, presets: &AttackPresets, out: &mut Vec<Issue>) {
        for (key, step) in [("main", &b.main), ("smash", &b.smash)] {
            let Some(step) = step else { continue };
            let at = format!("{at}.{key}");
            if let Some(hold) = &step.hold {
                if matches!(step.attack, AttackRef::Preset { .. })
                    && phases(&step.attack, presets).is_some_and(|p| !holds_after_active(&p))
                {
                    out.push(Issue {
                        path: format!("{at}.attack"),
                        message: "a held step's attack has a recovery with no spawn after its last active phase, where the hold begins".into(),
                    });
                }
                if matches!(hold.loop_attack, AttackRef::Preset { .. })
                    && phases(&hold.loop_attack, presets)
                        .is_some_and(|p| p.iter().any(|q| q.kind != PhaseKind::Active))
                {
                    out.push(Issue {
                        path: format!("{at}.hold.loop"),
                        message: "a loop's phases are all active".into(),
                    });
                }
            }
            walk(&step.next, &format!("{at}.next"), presets, out);
        }
    }
    let mut out = Vec::new();
    walk(&weapon.combo, "$.combo", presets, &mut out);
    walk(&weapon.dash, "$.dash", presets, &mut out);
    out
}

/// The highest argument number an attack's phases read, if they read any.
pub fn highest_arg(phases: &[AttackPhase]) -> Option<u8> {
    phases
        .iter()
        .map(|p| nodes_highest_arg(&p.spawn))
        .max()
        .flatten()
}

/// The highest argument number a list of nodes reads, if it reads any.
fn nodes_highest_arg(nodes: &[Emitter]) -> Option<u8> {
    fn expr(e: &Expr) -> Option<u8> {
        match e {
            Expr::Arg(i) => Some(*i),
            Expr::LoopIndex | Expr::PhaseTime | Expr::Const(_) => None,
            Expr::Rand(b)
            | Expr::Add(b)
            | Expr::Sub(b)
            | Expr::Mul(b)
            | Expr::Div(b)
            | Expr::Min(b)
            | Expr::Max(b) => expr(&b.0).max(expr(&b.1)),
            Expr::Clamp(b) => expr(&b.0).max(expr(&b.1)).max(expr(&b.2)),
        }
    }
    fn direction(d: &Direction) -> Option<u8> {
        match d {
            Direction::Aimed { offset } => expr(offset),
            Direction::Absolute { angle } | Direction::Relative { angle } => expr(angle),
            Direction::Sequential { step } => expr(step),
        }
    }
    nodes
        .iter()
        .map(|n| match n {
            Emitter::Repeat {
                count,
                interval_ticks,
                body,
            } => expr(count)
                .max(expr(interval_ticks))
                .max(nodes_highest_arg(body)),
            Emitter::Ring { count, body } => expr(count).max(nodes_highest_arg(body)),
            Emitter::Spread { count, angle, body } => {
                expr(count).max(expr(angle)).max(nodes_highest_arg(body))
            }
            Emitter::Rotate { angle, body } => expr(angle).max(nodes_highest_arg(body)),
            Emitter::Wait { ticks } => expr(ticks),
            Emitter::Orb {
                speed,
                direction: d,
                ticks,
                burst,
                stop_after,
                ..
            } => expr(speed)
                .max(expr(ticks))
                .max(stop_after.as_ref().and_then(expr))
                .max(direction(d))
                .max(nodes_highest_arg(burst)),
            Emitter::Zone {
                direction: d,
                wind_up_ticks,
                active_ticks,
                ..
            } => expr(wind_up_ticks)
                .max(active_ticks.as_ref().and_then(expr))
                .max(direction(d)),
        })
        .max()
        .flatten()
}

fn pair(b: &(Expr, Expr), args: &[i32]) -> Box<(Expr, Expr)> {
    Box::new((expr(&b.0, args), expr(&b.1, args)))
}

fn expr(e: &Expr, args: &[i32]) -> Expr {
    match e {
        Expr::Arg(i) => Expr::Const(args.get(usize::from(*i)).copied().unwrap_or(0)),
        Expr::LoopIndex | Expr::PhaseTime | Expr::Const(_) => e.clone(),
        Expr::Rand(b) => Expr::Rand(pair(b, args)),
        Expr::Add(b) => Expr::Add(pair(b, args)),
        Expr::Sub(b) => Expr::Sub(pair(b, args)),
        Expr::Mul(b) => Expr::Mul(pair(b, args)),
        Expr::Div(b) => Expr::Div(pair(b, args)),
        Expr::Min(b) => Expr::Min(pair(b, args)),
        Expr::Max(b) => Expr::Max(pair(b, args)),
        Expr::Clamp(b) => Expr::Clamp(Box::new((
            expr(&b.0, args),
            expr(&b.1, args),
            expr(&b.2, args),
        ))),
    }
}

fn direction(d: &Direction, args: &[i32]) -> Direction {
    match d {
        Direction::Aimed { offset } => Direction::Aimed {
            offset: expr(offset, args),
        },
        Direction::Absolute { angle } => Direction::Absolute {
            angle: expr(angle, args),
        },
        Direction::Relative { angle } => Direction::Relative {
            angle: expr(angle, args),
        },
        Direction::Sequential { step } => Direction::Sequential {
            step: expr(step, args),
        },
    }
}

fn emitters(nodes: &[Emitter], args: &[i32]) -> Vec<Emitter> {
    nodes
        .iter()
        .map(|n| match n {
            Emitter::Repeat {
                count,
                interval_ticks,
                body,
            } => Emitter::Repeat {
                count: expr(count, args),
                interval_ticks: expr(interval_ticks, args),
                body: emitters(body, args),
            },
            Emitter::Ring { count, body } => Emitter::Ring {
                count: expr(count, args),
                body: emitters(body, args),
            },
            Emitter::Spread { count, angle, body } => Emitter::Spread {
                count: expr(count, args),
                angle: expr(angle, args),
                body: emitters(body, args),
            },
            Emitter::Rotate { angle, body } => Emitter::Rotate {
                angle: expr(angle, args),
                body: emitters(body, args),
            },
            Emitter::Wait { ticks } => Emitter::Wait {
                ticks: expr(ticks, args),
            },
            Emitter::Orb {
                speed,
                direction: d,
                anchor,
                offset,
                style,
                ticks,
                breakable,
                burst,
                stop_after,
                link,
            } => Emitter::Orb {
                speed: expr(speed, args),
                direction: direction(d, args),
                anchor: *anchor,
                offset: *offset,
                style: style.clone(),
                ticks: expr(ticks, args),
                breakable: *breakable,
                burst: emitters(burst, args),
                stop_after: stop_after.as_ref().map(|e| expr(e, args)),
                link: link.clone(),
            },
            Emitter::Zone {
                shape,
                anchor,
                offset,
                direction: d,
                wind_up_ticks,
                active_ticks,
                warn_only,
            } => Emitter::Zone {
                shape: shape.clone(),
                anchor: *anchor,
                offset: *offset,
                direction: direction(d, args),
                wind_up_ticks: expr(wind_up_ticks, args),
                active_ticks: active_ticks.as_ref().map(|e| expr(e, args)),
                warn_only: *warn_only,
            },
        })
        .collect()
}
