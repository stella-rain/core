//! Attack presets (ADR-036 tier 3, ADR-055): the developer's attacks, written in the same
//! format as the attacks creators write inline, and registered as content IDs (ADR-026). A stage
//! uses one by ID with arguments, or holds an inline copy. **Taking a preset apart** copies its
//! definition inline with each argument written in as a constant; the stage then edits it
//! freely and no longer depends on the ID. The engine runs a preset by that same expansion, so a preset and
//! its inline copy run the same program and reach the same state hashes.
//!
//! Weapon presets (ADR-070) are the developer's weapons, each with its Main role (a combo tree)
//! and its Sub role (a combo-part bonus), written the same way; a stage uses one by ID, without
//! arguments, or holds an inline weapon.
//!
//! Under version 0 (ADR-035) these definitions are placeholders and change in place.

use std::sync::{Arc, OnceLock};

use crate::attack::{AttackPhase, AttackRef, Direction, Emitter, Expr};
use crate::registry::{self, Kind};
use crate::weapon::{MainWeapon, SubWeapon, WeaponPreset, WeaponRef};

/// `(id, definition)`: the definitions are JSON, the stage format's own, with `{"arg": n}` for
/// the preset's arguments. Angles are in 1/256 degree, lengths in 1/256 pixel and speeds in 1/256
/// pixel a tick. The bullet patterns are one active phase of one tick that spawns its bullets;
/// the melee swings (`slam`, `slash`, `thrust`) are a wind-up, an active hit box and a recovery.
const DEFINITIONS: &[(&str, &str)] = &[
    // One bullet at the target; argument 0 is its speed.
    (
        "aimed_single",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"fire","speed":{"arg":0},"direction":{"type":"aimed","offset":0}}]}]"#,
    ),
    // Seven bullets in a 30 degree fan at the target; argument 0 is their speed.
    (
        "aimed_spread",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":7,"angle":7680,"body":[
            {"type":"fire","speed":{"arg":0},"direction":{"type":"aimed","offset":0}}]}]}]"#,
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
    // Forty bullets, one every 3 ticks, each turned 6 degrees from the one before; argument 0
    // is their speed.
    (
        "spiral",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"repeat","count":40,"interval_ticks":3,"body":[
            {"type":"fire","speed":{"arg":0},"direction":{"type":"sequential","step":1536}}]}]}]"#,
    ),
    // Five bullets in a 20 degree fan at the target.
    (
        "spread_5",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":5,"angle":5120,"body":[
            {"type":"fire","speed":1536,"direction":{"type":"aimed","offset":0}}]}]}]"#,
    ),
    // A reach forward: a 12-pixel circle 28 pixels ahead; Pierce.
    (
        "thrust",
        r#"[{"kind":"wind_up","ticks":6},
            {"kind":"active","ticks":3,"type":"pierce",
             "hit_box":{"type":"circle","offset":{"forward":7168},"radius":3072}},
            {"kind":"recovery","ticks":12}]"#,
    ),
    // Two bullets side by side, 5 degrees apart, at the target.
    (
        "twin_shot",
        r#"[{"kind":"active","ticks":1,"spawn":[
            {"type":"spread","count":2,"angle":1280,"body":[
            {"type":"fire","speed":2048,"direction":{"type":"aimed","offset":0}}]}]}]"#,
    ),
];

/// `(id, definition)` of the weapon presets, in the stage format's own JSON. Placeholders until
/// the game design's weapons are written (Giant Sword and the others, app#209).
const WEAPONS: &[(&str, &str)] = &[
    // Main: slash, slash, then a thrust that charges (a thrust at level 1, at level 2 a blow all
    // round, `slam`'s, but a Slash: the charged slash), or a thrust after the second slash for B;
    // B from the root thrusts, then slashes; a dash-strike thrusts; assist looks 160 pixels away.
    // Slashes are Slash and thrusts Pierce (ADR-100). Sub: pulls what a B step hits by up to 24 pixels
    // over 6 ticks.
    (
        "blade",
        r#"{"main":{"combo":{
                "main":{"attack":{"preset":{"id":"slash"}},"next":{
                    "main":{"attack":{"preset":{"id":"slash"}},"next":{
                        "main":{"attack":{"preset":{"id":"thrust"}},
                                "charge":{"max_ticks":90,"move_pct":50,"levels":[
                                    {"at_ticks":20,"perfect_ticks":6,"perfect_pct":50,
                                     "attack":{"preset":{"id":"thrust"}}},
                                    {"at_ticks":50,"perfect_ticks":6,"perfect_pct":100,
                                     "attack":{"inline":[
                                        {"kind":"wind_up","ticks":12,"move_pct":50},
                                        {"kind":"active","ticks":4,"move_pct":50,"type":"slash",
                                         "hit_box":{"type":"circle","radius":10240}},
                                        {"kind":"recovery","ticks":20,"move_pct":50}]}}]}},
                        "sub":{"attack":{"preset":{"id":"thrust"}}}}}}},
                "sub":{"attack":{"preset":{"id":"thrust"}},"next":{
                    "sub":{"attack":{"preset":{"id":"slash"}}}}}},
            "dash":{"main":{"attack":{"preset":{"id":"thrust"}}}},
            "window_ticks":20,"dash_strike_ticks":10,"assist_range":40960},
            "sub":{"bonus":{"type":"pull","distance":6144,"ticks":6}}}"#,
    ),
    // Main: slam, then a slam that charges, rooted (a slam at level 1); B swings `slash`'s arc,
    // but a Strike: every Hammer step is Strike (ADR-100); assist looks 128 pixels away. Sub: pulls what a B step hits by up to 40 pixels over 8 ticks.
    (
        "maul",
        r#"{"main":{"combo":{
                "main":{"attack":{"preset":{"id":"slam"}},"next":{
                    "main":{"attack":{"preset":{"id":"slam"}},
                            "charge":{"max_ticks":120,"move_pct":0,"levels":[
                                {"at_ticks":30,"perfect_ticks":8,"perfect_pct":100,
                                 "attack":{"preset":{"id":"slam"}}}]}}}},
                "sub":{"attack":{"inline":[
                    {"kind":"wind_up","ticks":4},
                    {"kind":"active","ticks":3,"type":"strike",
                     "hit_box":{"type":"arc","offset":{"forward":2048},"radius":8192,
                                "half_angle":15360}},
                    {"kind":"recovery","ticks":10}]}}},
            "window_ticks":24,"assist_range":32768},
            "sub":{"bonus":{"type":"pull","distance":10240,"ticks":8}}}"#,
    ),
];

fn parsed_weapons() -> &'static [(&'static str, WeaponPreset)] {
    static PARSED: OnceLock<Vec<(&'static str, WeaponPreset)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        WEAPONS
            .iter()
            .map(|(id, json)| {
                let preset = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("the built-in weapon {id} does not parse: {e}"));
                (*id, preset)
            })
            .collect()
    })
}

/// Every preset's source as `(kind, id, definition)`, attacks (`b'a'`) then weapons (`b'w'`),
/// each in its list's order: what the run replay's content hash covers (ADR-051).
pub(crate) fn sources() -> impl Iterator<Item = (u8, &'static str, &'static str)> {
    let attacks = DEFINITIONS.iter().map(|(id, json)| (b'a', *id, *json));
    attacks.chain(WEAPONS.iter().map(|(id, json)| (b'w', *id, *json)))
}

/// The ID of every weapon preset that has a definition.
pub fn weapon_ids() -> impl Iterator<Item = &'static str> {
    WEAPONS.iter().map(|(id, _)| *id)
}

/// A weapon preset's definition, both roles.
pub fn weapon_preset(id: &str) -> Option<&'static WeaponPreset> {
    parsed_weapons()
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, w)| w)
}

/// The Main weapon a reference names: the preset's Main role, or the inline weapon itself.
/// `None` for a preset that has no definition.
pub fn main_weapon(weapon: &WeaponRef<MainWeapon>) -> Option<&MainWeapon> {
    match weapon {
        WeaponRef::Preset { id } => weapon_preset(&id.0).map(|p| &p.main),
        WeaponRef::Inline(w) => Some(w),
    }
}

/// The Sub weapon a reference names, as `main_weapon` does.
pub fn sub_weapon(weapon: &WeaponRef<SubWeapon>) -> Option<&SubWeapon> {
    match weapon {
        WeaponRef::Preset { id } => weapon_preset(&id.0).map(|p| &p.sub),
        WeaponRef::Inline(w) => Some(w),
    }
}

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
pub fn take_apart(id: &str, args: &[i32]) -> Option<Vec<AttackPhase>> {
    let arity = registry::find(registry::ENTRIES, Kind::Preset, id)?.args;
    if usize::from(arity) != args.len() {
        return None;
    }
    Some(
        definition(id)?
            .iter()
            .map(|p| AttackPhase {
                spawn: emitters(&p.spawn, args),
                ..p.clone()
            })
            .collect(),
    )
}

/// The phases an attack runs: a preset taken apart, or the inline phases themselves.
pub fn phases(attack: &AttackRef) -> Option<Arc<Vec<AttackPhase>>> {
    match attack {
        AttackRef::Preset { id, args } => take_apart(&id.0, args).map(Arc::new),
        AttackRef::Inline(phases) => Some(Arc::new(phases.clone())),
    }
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
            Emitter::Fire {
                speed,
                direction: d,
                ..
            } => expr(speed).max(direction(d)),
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
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => expr(after_ticks)
                .max(nodes_highest_arg(body))
                .max(nodes_highest_arg(then)),
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
            Emitter::Fire {
                speed,
                direction: d,
                bullet,
                ticks,
            } => Emitter::Fire {
                speed: expr(speed, args),
                direction: direction(d, args),
                bullet: bullet.clone(),
                ticks: ticks.as_ref().map(|t| expr(t, args)),
            },
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
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => Emitter::OnBullet {
                after_ticks: expr(after_ticks, args),
                body: emitters(body, args),
                then: emitters(then, args),
            },
        })
        .collect()
}
