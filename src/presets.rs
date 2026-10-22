//! Attack presets (ADR-036 tier 3, ADR-055): the developer's attacks, written in the same
//! format as the attacks creators write inline, and registered as content IDs (ADR-026). A stage
//! uses one by ID with arguments, or holds an inline copy. **Taking a preset apart** copies its
//! definition inline with each argument written in as a constant; the stage then edits it
//! freely and no longer depends on the ID. The engine runs a preset by that same expansion, so a preset and
//! its inline copy run the same program and reach the same state hashes.
//!
//! Weapon presets (ADR-084) are the developer's weapons, each a combo tree with its smash effect,
//! written the same way; a stage uses one by ID, without arguments, or holds an inline weapon.
//!
//! Under version 0 (ADR-035) these definitions are placeholders and change in place.

use std::sync::{Arc, OnceLock};

use crate::attack::{AttackPhase, AttackRef, Direction, Emitter, Expr};
use crate::registry::{self, Kind};
use crate::weapon::{Weapon, WeaponRef};

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

/// `(id, trees)` of the weapon presets: each weapon's trees in order, tree 1 first (ADR-117), as
/// `(tree ID, definition)` in the stage format's own JSON. A tree ID is lowercase snake_case, a
/// placeholder name: `<weapon>_base` for tree 1. Trees 2 and 3 are added per weapon as content.
/// Placeholders until the game design's weapons are written (Giant Sword and the others,
/// app#209).
const WEAPONS: &[(&str, &[(&str, &str)])] = &[
    // The Vindictus shape (ADR-084): A, AA and AAA are spins around the main character, each
    // wider than the last, and every Hammer step is Strike (ADR-100). AAA holds (ADR-084 point 6):
    // its loop spins on at half speed while A stays held, each loop a new attack that hits again
    // after 8 ticks, and the release is a heavy slam at 100 times the default damage (ADR-103). B is an overhead smash (`slam`) and AB a ground shock, a forward
    // arc; a smash ends the combo. The dash-strike is a short spin, the dash smash `slam`. Smashes
    // pull what they hit by up to 40 pixels over 8 ticks; assist looks 128 pixels away. AAB (an
    // arc) and AAAB (a wide circle) are Strike smashes at 100 times the default damage, placeholders,
    // opened by the permanent upgrades `giant_hammer_aab` and `giant_hammer_aaab` (ADR-084 point 7).
    (
        "giant_hammer",
        &[(
            "giant_hammer_base",
            r#"{"combo":{
                "main":{"attack":{"inline":[
                            {"kind":"wind_up","ticks":6},
                            {"kind":"active","ticks":4,"type":"strike",
                             "hit_box":{"type":"circle","radius":8192}},
                            {"kind":"recovery","ticks":14}]},
                        "next":{
                    "main":{"attack":{"inline":[
                                {"kind":"wind_up","ticks":6},
                                {"kind":"active","ticks":5,"type":"strike",
                                 "hit_box":{"type":"circle","radius":10240}},
                                {"kind":"recovery","ticks":14}]},
                            "next":{
                        "main":{"attack":{"inline":[
                                    {"kind":"wind_up","ticks":8},
                                    {"kind":"active","ticks":6,"type":"strike",
                                     "hit_box":{"type":"circle","radius":12288}},
                                    {"kind":"recovery","ticks":20}]},
                                "hold":{"loop":{"inline":[{"kind":"active","ticks":8,"rehit_ticks":8,"type":"strike","hit_box":{"type":"circle","radius":12288}}]},"release":{"inline":[{"kind":"wind_up","ticks":10},{"kind":"active","ticks":5,"type":"strike","damage":10000,"hit_box":{"type":"circle","offset":{"forward":4096},"radius":16384}},{"kind":"recovery","ticks":24}]},"max_ticks":180,"move_pct":50},"next":{
                            "main":{"attack":{"inline":[
                                        {"kind":"wind_up","ticks":8},
                                        {"kind":"active","ticks":6,"type":"strike",
                                         "hit_box":{"type":"circle","radius":14336}},
                                        {"kind":"recovery","ticks":20}]},
                                    "next":{
                                "main":{"attack":{"inline":[
                                            {"kind":"wind_up","ticks":10},
                                            {"kind":"active","ticks":6,"type":"strike",
                                             "hit_box":{"type":"circle","radius":16384}},
                                            {"kind":"recovery","ticks":24}]},
                                        "next":{"smash":{"attack":{"inline":[
                                            {"kind":"wind_up","ticks":22,"move_pct":50},
                                            {"kind":"active","ticks":6,"move_pct":50,"type":"strike","damage":16000,
                                             "hit_box":{"type":"circle","radius":22528}},
                                            {"kind":"recovery","ticks":30,"move_pct":50}]}}}},
                                "smash":{"attack":{"inline":[
                                    {"kind":"wind_up","ticks":18,"move_pct":50},
                                    {"kind":"active","ticks":5,"move_pct":50,"type":"strike","damage":12000,
                                     "hit_box":{"type":"circle","radius":18432}},
                                    {"kind":"recovery","ticks":28,"move_pct":50}]}}}},
                            "smash":{"attack":{"inline":[{"kind":"wind_up","ticks":16,"move_pct":50},{"kind":"active","ticks":5,"move_pct":50,"type":"strike","damage":10000,"hit_box":{"type":"circle","radius":16384}},{"kind":"recovery","ticks":26,"move_pct":50}]},"unlock":"giant_hammer_aaab"}}},"smash":{"attack":{"inline":[{"kind":"wind_up","ticks":12,"move_pct":50},{"kind":"active","ticks":4,"move_pct":50,"type":"strike","damage":10000,"hit_box":{"type":"arc","offset":{"forward":4096},"radius":14336,"half_angle":24576}},{"kind":"recovery","ticks":22,"move_pct":50}]},"unlock":"giant_hammer_aab"}}},
                    "smash":{"attack":{"inline":[
                                {"kind":"wind_up","ticks":14,"move_pct":50},
                                {"kind":"active","ticks":4,"move_pct":50,"type":"strike",
                                 "hit_box":{"type":"arc","offset":{"forward":4096},
                                            "radius":12288,"half_angle":20480}},
                                {"kind":"recovery","ticks":22,"move_pct":50}]}}}},
                "smash":{"attack":{"preset":{"id":"slam"}}}},
            "dash":{"main":{"attack":{"inline":[
                        {"kind":"wind_up","ticks":2},
                        {"kind":"active","ticks":3,"type":"strike",
                         "hit_box":{"type":"circle","radius":8192}},
                        {"kind":"recovery","ticks":8}]}},
                    "smash":{"attack":{"preset":{"id":"slam"}}}},
            "window_ticks":24,"dash_strike_ticks":10,"assist_range":32768,
            "smash":{"type":"stun","ticks":30}}"#,
        )],
    ),
    // The Vindictus shape (ADR-084): A, AA, AAA, AAAA and AAAAA are a slash, a slash, a thrust
    // (Pierce, ADR-100), a slash and a finishing thrust; AAA charges, at level 1 a wide slash and at level 2 a spin all round, each with
    // the +50% perfect bonus. B is a slash and AB a thrust, both smashes that end the combo; AAB, AAAB, AAAAB and AAAAAB
    // are smashes too, a wide Slash, a Pierce, a wider Slash and a long Pierce (AAAAAB the
    // strongest, at 160 times the default damage), the numbers placeholders. The
    // dash-strike is a lunge thrust, the dash smash a slash. Assist looks 160 pixels away.
    // Smashes put the bleed on what they hit, the bleed rune's numbers (ADR-084 point 8). AAB (a
    // wide Slash) and AAAB (a long Pierce) are smashes at 100 times the default damage,
    // placeholders, opened by the permanent upgrades `giant_sword_aab` and `giant_sword_aaab`
    // (ADR-084 point 7).
    (
        "giant_sword",
        &[(
            "giant_sword_base",
            r#"{"combo":{
                "main":{"attack":{"preset":{"id":"slash"}},"next":{
                    "main":{"attack":{"preset":{"id":"slash"}},"next":{
                        "main":{"attack":{"preset":{"id":"thrust"}},
                                "charge":{"max_ticks":90,"move_pct":50,"levels":[
                                    {"at_ticks":20,"perfect_ticks":6,"perfect_pct":50,
                                     "attack":{"inline":[
                                        {"kind":"wind_up","ticks":8,"move_pct":50},
                                        {"kind":"active","ticks":4,"move_pct":50,"type":"slash",
                                         "hit_box":{"type":"arc","offset":{"forward":2048},
                                                    "radius":8192,"half_angle":22528}},
                                        {"kind":"recovery","ticks":14,"move_pct":50}]}},
                                    {"at_ticks":50,"perfect_ticks":6,"perfect_pct":50,
                                     "attack":{"inline":[
                                        {"kind":"wind_up","ticks":12,"move_pct":50},
                                        {"kind":"active","ticks":4,"move_pct":50,"type":"slash",
                                         "hit_box":{"type":"circle","radius":10240}},
                                        {"kind":"recovery","ticks":20,"move_pct":50}]}}]},"next":{
                            "main":{"attack":{"preset":{"id":"slash"}},"next":{
                                "main":{"attack":{"preset":{"id":"thrust"}},"next":{
                                    "smash":{"attack":{"inline":[
                                        {"kind":"wind_up","ticks":12},
                                        {"kind":"active","ticks":4,"type":"pierce","damage":16000,
                                         "hit_box":{"type":"circle","offset":{"forward":10240},"radius":6144}},
                                        {"kind":"recovery","ticks":22}]}}}},
                                "smash":{"attack":{"inline":[
                                    {"kind":"wind_up","ticks":10},
                                    {"kind":"active","ticks":4,"type":"slash","damage":12000,
                                     "hit_box":{"type":"arc","offset":{"forward":2048},"radius":12288,"half_angle":24576}},
                                    {"kind":"recovery","ticks":16}]}}}},
                            "smash":{"attack":{"inline":[{"kind":"wind_up","ticks":10},{"kind":"active","ticks":3,"type":"pierce","damage":10000,"hit_box":{"type":"circle","offset":{"forward":8192},"radius":4096}},{"kind":"recovery","ticks":18}]},"unlock":"giant_sword_aaab"}}},"smash":{"attack":{"inline":[{"kind":"wind_up","ticks":8},{"kind":"active","ticks":4,"type":"slash","damage":10000,"hit_box":{"type":"arc","offset":{"forward":2048},"radius":10240,"half_angle":20480}},{"kind":"recovery","ticks":14}]},"unlock":"giant_sword_aab"}}},
                        "smash":{"attack":{"preset":{"id":"thrust"}}}}},
                "smash":{"attack":{"preset":{"id":"slash"}}}},
            "dash":{"main":{"attack":{"preset":{"id":"thrust"}}},
                    "smash":{"attack":{"preset":{"id":"slash"}}}},
            "window_ticks":20,"dash_strike_ticks":10,"assist_range":40960,
            "smash":{"type":"bleed"}}"#,
        )],
    ),
];

/// A weapon's trees, parsed: `(weapon ID, [(tree ID, definition)])`.
type ParsedTrees = Vec<(&'static str, Vec<(&'static str, Weapon)>)>;

fn parsed_weapons() -> &'static ParsedTrees {
    static PARSED: OnceLock<ParsedTrees> = OnceLock::new();
    PARSED.get_or_init(|| {
        WEAPONS
            .iter()
            .map(|(id, trees)| {
                let trees = trees
                    .iter()
                    .map(|(tree, json)| {
                        let weapon = serde_json::from_str(json).unwrap_or_else(|e| {
                            panic!("the built-in weapon tree {tree} does not parse: {e}")
                        });
                        (*tree, weapon)
                    })
                    .collect();
                (*id, trees)
            })
            .collect()
    })
}

/// Every preset's source as `(kind, id, definition)`, attacks (`b'a'`) then weapons (`b'w'`:
/// tree 1 under the weapon's ID, `b't'`: each other tree under its tree ID), each in its list's
/// order: what the run replay's content hash covers (ADR-051).
pub(crate) fn sources() -> impl Iterator<Item = (u8, &'static str, &'static str)> {
    let attacks = DEFINITIONS.iter().map(|(id, json)| (b'a', *id, *json));
    let weapons = WEAPONS.iter().flat_map(|(id, trees)| {
        trees.iter().enumerate().map(move |(i, (tree, json))| {
            if i == 0 {
                (b'w', *id, *json)
            } else {
                (b't', *tree, *json)
            }
        })
    });
    attacks.chain(weapons)
}

/// The ID of every weapon preset that has a definition.
pub fn weapon_ids() -> impl Iterator<Item = &'static str> {
    WEAPONS.iter().map(|(id, _)| *id)
}

/// The IDs of a weapon's trees, tree 1 first (ADR-117); none for an ID that is no weapon.
pub fn tree_ids(weapon: &str) -> impl Iterator<Item = &'static str> {
    WEAPONS
        .iter()
        .find(|(id, _)| *id == weapon)
        .into_iter()
        .flat_map(|(_, trees)| trees.iter().map(|(tree, _)| *tree))
}

/// What is wrong with naming `tree` for the weapon `weapon`, if anything (ADR-020: fail closed,
/// and never echo the file's text): not a well-formed ID, or not one of the weapon's trees. A
/// weapon with no definition is left to the weapon's own check.
pub fn tree_problem(weapon: &str, tree: &str) -> Option<&'static str> {
    if !registry::is_well_formed(tree) {
        Some("not a valid ID (lowercase snake_case, at most 32 characters)")
    } else if tree_ids(weapon).next().is_some() && !tree_ids(weapon).any(|t| t == tree) {
        Some("no such tree for this weapon")
    } else {
        None
    }
}

/// A weapon preset's definition: its first tree.
pub fn weapon_preset(id: &str) -> Option<&'static Weapon> {
    weapon_tree(id, None)
}

/// A weapon's tree: the one `tree` names, or the first when `None`. `None` for a weapon with no
/// definition or a tree it does not have.
pub fn weapon_tree(id: &str, tree: Option<&str>) -> Option<&'static Weapon> {
    let (_, trees) = parsed_weapons().iter().find(|(i, _)| *i == id)?;
    match tree {
        None => trees.first(),
        Some(tree) => trees.iter().find(|(t, _)| *t == tree),
    }
    .map(|(_, w)| w)
}

/// The weapon a reference names: the preset's definition (in its tree), or the inline weapon
/// itself. `None` for a preset that has no definition, or a tree it does not have.
pub fn weapon(weapon: &WeaponRef<Weapon>) -> Option<&Weapon> {
    match weapon {
        WeaponRef::Preset { id, tree } => weapon_tree(&id.0, tree.as_ref().map(|t| t.0.as_str())),
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
