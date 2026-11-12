//! The attack checks: phases, dashes, hit boxes, emitters, loops and expressions.

use super::limits;
use super::limits::*;
use super::{Ctx, Foe};
use crate::attack::{
    AttackPhase, AttackRef, Dash, Direction, Emitter, Expr, HitBox, Offset, PhaseKind, ZoneShape,
};
use crate::registry::Kind;
use crate::trig;

impl<'a> Ctx<'a> {
    /// Validates an attack reference: either a preset with arguments or an inline attack.
    /// A preset's arguments are bounded in code by its registry entry (ADR-CONT-3, ADR-CONT-2).
    pub(super) fn attack_ref(&mut self, a: &AttackRef) {
        match a {
            AttackRef::Preset { id, args } => {
                self.key("preset", |c| {
                    let entry = c.content_id("id", id, Kind::Preset);
                    if !c.at_most("args", args.len(), MAX_PRESET_ARGS, "arguments") {
                        return;
                    }
                    if let Some(e) = entry {
                        if usize::from(e.args) != args.len() {
                            c.key("args", |c| {
                                c.fail(format!(
                                    "this preset takes {} arguments (found {})",
                                    e.args,
                                    args.len()
                                ))
                            });
                        }
                    }
                    c.key("args", |c| {
                        for (i, v) in args.iter().enumerate() {
                            c.idx(i, |c| {
                                c.number(*v);
                                if let Some(e) = entry {
                                    if usize::from(e.args) == args.len()
                                        && (-MAX_NUMBER..=MAX_NUMBER).contains(v)
                                    {
                                        let b = &e.arg_bounds[i];
                                        if *v < b.min {
                                            c.fail(format!(
                                                "preset {} argument {} ({}) is {}, below its minimum {}",
                                                e.id, i, b.name, v, b.min
                                            ));
                                        } else if *v > b.max {
                                            c.fail(format!(
                                                "preset {} argument {} ({}) is {}, above its maximum {}",
                                                e.id, i, b.name, v, b.max
                                            ));
                                        }
                                    }
                                }
                            });
                        }
                    });
                });
            }
            AttackRef::Inline(phases) => self.key("inline", |c| c.phases(phases)),
        }
    }

    /// An attack's phases. The nodes they spawn count against one pattern budget.
    pub(super) fn phases(&mut self, phases: &[AttackPhase]) {
        if phases.is_empty() {
            self.fail("an attack has at least one phase");
            return;
        }
        if !self.at_most_here(phases.len(), MAX_ATTACK_PHASES, "phases") {
            return;
        }
        let mut count = 0;
        for (i, p) in phases.iter().enumerate() {
            let going = self.idx(i, |c| {
                c.attack_phase(p);
                c.key("spawn", |c| c.emitters(&p.spawn, 1, &mut count))
            });
            if !going {
                return;
            }
        }
    }

    /// One phase: its ticks, movement, hit box, damage and re-hits are bounded. A re-hit needs
    /// an active phase with a hit box, since nothing else hits.
    fn attack_phase(&mut self, p: &AttackPhase) {
        self.within("ticks", i64::from(p.ticks), 1, i64::from(MAX_PHASE_TICKS));
        self.within("move_pct", i64::from(p.move_pct), 0, 100);
        if let Some(h) = &p.hit_box {
            self.key("hit_box", |c| c.hit_box(h));
        }
        if let Some(d) = p.damage {
            self.within("damage", i64::from(d), 1, i64::from(MAX_DAMAGE));
        }
        if let Some(d) = &p.dash {
            self.key("dash", |c| c.dash(p, d));
        }
        if let Some(d) = p.poise_damage {
            let refusal = if p.kind != PhaseKind::Active || p.hit_box.is_none() {
                Some("only an active phase with a hit box deals poise damage")
            } else if self.foe != Foe::Other {
                Some("a main character's or an ally's attack only")
            } else if self.sim_version < 2 {
                Some("needs sim_version 2")
            } else {
                None
            };
            match refusal {
                Some(why) => self.key("poise_damage", |c| c.fail(why)),
                None => self.within("poise_damage", i64::from(d), 0, i64::from(MAX_POISE_DAMAGE)),
            }
        }
        if let Some(d) = p.stagger {
            let refusal = if p.kind != PhaseKind::Active || p.hit_box.is_none() {
                Some("only an active phase with a hit box adds stagger")
            } else if self.foe != Foe::Other {
                Some("a main character's or an ally's attack only")
            } else if self.sim_version < 2 {
                Some("needs sim_version 2")
            } else {
                None
            };
            match refusal {
                Some(why) => self.key("stagger", |c| c.fail(why)),
                None => self.within(
                    "stagger",
                    i64::from(d),
                    0,
                    i64::from(crate::limits::MAX_STAGGER),
                ),
            }
        }
        if let Some(a) = p.advance {
            self.key("advance", |c| c.advance(a));
        }
        if let Some(r) = p.rehit_ticks {
            if p.kind != PhaseKind::Active || p.hit_box.is_none() {
                self.key("rehit_ticks", |c| {
                    c.fail("only an active phase with a hit box hits again")
                });
            } else {
                self.within("rehit_ticks", i64::from(r), 1, i64::from(MAX_PHASE_TICKS));
            }
        }
    }

    /// An advance is the main character's: not an enemy's or the boss's attack, `sim_version` 2
    /// only, 1 pixel to `MAX_ADVANCE_DISTANCE`.
    fn advance(&mut self, a: crate::fixed::Fx) {
        if self.foe != Foe::Other {
            self.fail("an advance is the main character's combo step, not an enemy's or the boss's attack");
        }
        if self.sim_version < 2 {
            self.fail("needs sim_version 2");
        }
        let (least, most) = (
            i64::from(crate::fixed::Fx::px(1).0),
            i64::from(crate::fixed::Fx::px(crate::limits::MAX_ADVANCE_DISTANCE).0),
        );
        if !(least..=most).contains(&i64::from(a.0)) {
            self.fail(format!(
                "must be between {least} and {most} (found {})",
                a.0
            ));
        }
    }

    /// A dash is an active phase's, an enemy's or the boss's, as long as 1 pixel to
    /// `MAX_DASH_DISTANCE`, and the phase does not also walk.
    fn dash(&mut self, p: &AttackPhase, d: &Dash) {
        if p.kind != PhaseKind::Active {
            self.fail("only an active phase dashes");
        }
        if self.foe == Foe::Other {
            self.fail(
                "a dash is an enemy's or the boss's attack, not an ally's or the main character's",
            );
        }
        if p.move_pct != 0 {
            self.fail("a dashing phase has a move_pct of 0, or its walk adds to the dash");
        }
        self.within(
            "distance",
            i64::from(d.distance.0),
            i64::from(crate::fixed::Fx::px(1).0),
            i64::from(crate::fixed::Fx::px(limits::MAX_DASH_DISTANCE).0),
        );
    }

    /// A hit box's radius, length, width and offset are at most `MAX_RADIUS`, and an arc's half
    /// angle is 0 to 180 degrees.
    fn hit_box(&mut self, h: &HitBox) {
        match h {
            HitBox::Circle { offset, radius, .. } => {
                self.within("radius", i64::from(radius.0), 1, i64::from(MAX_RADIUS));
                self.offset(offset);
            }
            HitBox::Arc {
                offset,
                radius,
                half_angle,
                ..
            } => {
                self.within("radius", i64::from(radius.0), 1, i64::from(MAX_RADIUS));
                self.offset(offset);
                self.half_angle(*half_angle);
            }
            HitBox::Line {
                offset,
                length,
                width,
                ..
            } => {
                self.within("length", i64::from(length.0), 1, i64::from(MAX_RADIUS));
                self.within("width", i64::from(width.0), 1, i64::from(MAX_RADIUS));
                self.offset(offset);
            }
        }
    }

    /// An orb's or a zone's direction: its expression is checked.
    fn direction(&mut self, direction: &Direction) {
        self.key("direction", |c| match direction {
            Direction::Aimed { offset } => c.key("offset", |c| c.expr(offset)),
            Direction::Absolute { angle } | Direction::Relative { angle } => {
                c.key("angle", |c| c.expr(angle))
            }
            Direction::Sequential { step } => c.key("step", |c| c.expr(step)),
        });
    }

    fn offset(&mut self, offset: &Offset) {
        let r = i64::from(MAX_RADIUS);
        self.key("offset", |c| {
            c.within("forward", i64::from(offset.forward.0), -r, r);
            c.within("right", i64::from(offset.right.0), -r, r);
        });
    }

    fn half_angle(&mut self, half_angle: i32) {
        self.within(
            "half_angle",
            i64::from(half_angle),
            0,
            i64::from(180 * trig::PER_DEGREE),
        );
    }

    /// A zone's shape, as a hit box's, and the ticks it warns and hurts for.
    fn zone_shape(&mut self, shape: &ZoneShape) {
        let r = i64::from(MAX_RADIUS);
        match shape {
            ZoneShape::Circle { radius } => self.within("radius", i64::from(radius.0), 1, r),
            ZoneShape::Arc { radius, half_angle } => {
                self.within("radius", i64::from(radius.0), 1, r);
                self.half_angle(*half_angle);
            }
            ZoneShape::Line { length, width } => {
                self.within("length", i64::from(length.0), 1, r);
                self.within("width", i64::from(width.0), 1, r);
            }
        }
    }

    /// A zone's tick count: a constant must be in `lo..=MAX_ZONE_TICKS`; a computed one is
    /// clamped by the engine.
    fn zone_ticks(&mut self, key: &'static str, e: &Expr, lo: u32) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), i64::from(lo), i64::from(MAX_ZONE_TICKS));
        }
    }

    fn number(&mut self, v: i32) {
        self.within(
            "value",
            i64::from(v),
            -i64::from(MAX_NUMBER),
            i64::from(MAX_NUMBER),
        );
    }

    /// Checks a list of emitters at `depth`, counting nodes in `count`. Returns false once a
    /// limit is hit, so that the rest of the tree is not walked.
    fn emitters(&mut self, list: &[Emitter], depth: u32, count: &mut u32) -> bool {
        for (i, e) in list.iter().enumerate() {
            *count += 1;
            if depth > MAX_PATTERN_DEPTH {
                self.idx(i, |c| {
                    c.fail(format!(
                        "a pattern is at most {MAX_PATTERN_DEPTH} levels deep"
                    ))
                });
                return false;
            }
            if *count > MAX_PATTERN_NODES {
                self.idx(i, |c| {
                    c.fail(format!("a pattern has at most {MAX_PATTERN_NODES} nodes"))
                });
                return false;
            }
            if !self.idx(i, |c| c.emitter(e, depth, count)) {
                return false;
            }
        }
        true
    }

    fn emitter(&mut self, e: &Emitter, depth: u32, count: &mut u32) -> bool {
        match e {
            Emitter::Repeat {
                count: n,
                interval_ticks,
                body,
            } => {
                self.constant_ticks("interval_ticks", interval_ticks);
                self.looped(n, body, depth, count)
            }
            Emitter::Ring { count: n, body } => self.looped(n, body, depth, count),
            Emitter::Spread {
                count: n,
                angle,
                body,
            } => {
                self.key("angle", |c| c.expr(angle));
                self.looped(n, body, depth, count)
            }
            Emitter::Rotate { angle, body } => {
                self.key("angle", |c| c.expr(angle));
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Wait { ticks } => {
                self.constant_ticks("ticks", ticks);
                true
            }
            Emitter::Orb {
                speed,
                direction,
                style,
                ticks,
                burst,
                stop_after,
                link,
                offset,
                ..
            } => {
                self.offset(offset);
                if self.foe == Foe::Other {
                    self.fail("an orb is an enemy's attack, not an ally's or the main character's");
                }
                self.key("speed", |c| c.expr(speed));
                self.direction(direction);
                if let Some(b) = style {
                    self.content_id("style", b, Kind::Orb);
                }
                self.key("ticks", |c| c.expr(ticks));
                if let Expr::Const(v) = ticks {
                    self.within("ticks", i64::from(*v), 1, i64::from(limits::MAX_ORB_TICKS));
                }
                if let Some(n) = stop_after {
                    self.key("stop_after", |c| c.expr(n));
                    if let Expr::Const(v) = n {
                        let most = match ticks {
                            Expr::Const(t) => i64::from(*t),
                            _ => i64::from(limits::MAX_ORB_TICKS),
                        };
                        self.within("stop_after", i64::from(*v), 0, most);
                    }
                }
                if let Some(l) = link {
                    self.key("link", |c| c.link(l));
                }
                self.key("burst", |c| {
                    c.burst_nodes(burst);
                    c.emitters(burst, depth + 1, count)
                })
            }
            Emitter::Zone {
                shape,
                offset,
                direction,
                wind_up_ticks,
                active_ticks,
                warn_only,
                ..
            } => {
                self.key("shape", |c| c.zone_shape(shape));
                self.offset(offset);
                self.direction(direction);
                match (warn_only, active_ticks) {
                    (true, Some(_)) => {
                        self.fail("a warn_only zone has no active_ticks: it never hurts")
                    }
                    (false, None) => self.fail("a zone needs active_ticks unless it is warn_only"),
                    _ => {}
                }
                self.zone_ticks("wind_up_ticks", wind_up_ticks, u32::from(*warn_only));
                if let Some(a) = active_ticks {
                    self.zone_ticks("active_ticks", a, 1);
                }
                true
            }
        }
    }

    /// The body of a `repeat`, `ring` or `spread` of `n` rounds.
    fn looped(&mut self, n: &Expr, body: &[Emitter], depth: u32, count: &mut u32) -> bool {
        self.repeat_count("count", n);
        let rounds = match n {
            Expr::Const(v) => i64::from(*v),
            _ => i64::MAX,
        };
        let outer = std::mem::replace(&mut self.loop_rounds, rounds);
        let ok = self.key("body", |c| c.emitters(body, depth + 1, count));
        self.loop_rounds = outer;
        ok
    }

    /// An orb's `link`: a beam 1 to 64 pixels wide, hitting every 1 to 600 ticks, between the
    /// orbs of a loop of at least 2 rounds.
    fn link(&mut self, l: &crate::attack::Link) {
        if self.loop_rounds < 2 {
            self.fail("a link joins the orbs of a ring, spread or repeat of at least 2 rounds");
        }
        self.within(
            "width",
            i64::from(l.width.0),
            1,
            i64::from(crate::fixed::Fx::px(64).0),
        );
        self.within("rehit_ticks", i64::from(l.rehit_ticks), 1, 600);
    }

    /// An orb's burst holds only the nodes that place zones and time them: `zone`, `ring`,
    /// `spread`, `repeat`, `rotate` and `wait`.
    fn burst_nodes(&mut self, list: &[Emitter]) {
        for (i, e) in list.iter().enumerate() {
            self.idx(i, |c| match e {
                Emitter::Orb { .. } => {
                    c.fail("a burst holds only zone, ring, spread, repeat, rotate and wait nodes")
                }
                Emitter::Repeat { body, .. }
                | Emitter::Ring { body, .. }
                | Emitter::Spread { body, .. }
                | Emitter::Rotate { body, .. } => c.key("body", |c| c.burst_nodes(body)),
                Emitter::Wait { .. } | Emitter::Zone { .. } => {}
            });
        }
    }

    /// A repeat, ring or spread count: a constant must be in `1..=MAX_REPEAT`; a computed one
    /// is clamped by the engine.
    fn repeat_count(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 1, i64::from(MAX_REPEAT));
        }
    }

    /// A tick count: a constant must be in `0..=MAX_TICKS`; a computed one is clamped.
    fn constant_ticks(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 0, i64::from(MAX_TICKS));
        }
    }

    /// An expression of at most `MAX_EXPR_NODES` nodes. An inline attack in a stage takes no
    /// arguments, so `arg` is not valid there; presets, which do, are checked with `validate_attack`.
    fn expr(&mut self, e: &Expr) {
        let mut nodes = 0;
        self.expr_node(e, self.arity, &mut nodes);
    }

    fn expr_node(&mut self, e: &Expr, args: u8, nodes: &mut u32) {
        *nodes += 1;
        if *nodes > MAX_EXPR_NODES {
            if *nodes == MAX_EXPR_NODES + 1 {
                self.fail(format!("an expression has at most {MAX_EXPR_NODES} nodes"));
            }
            return;
        }
        match e {
            Expr::Arg(i) => {
                if *i >= args {
                    self.fail("there is no such argument in an inline attack");
                }
            }
            Expr::LoopIndex | Expr::PhaseTime => {}
            Expr::Const(v) => self.number(*v),
            Expr::Rand(b) => self.pair("rand", b, args, nodes),
            Expr::Add(b) => self.pair("add", b, args, nodes),
            Expr::Sub(b) => self.pair("sub", b, args, nodes),
            Expr::Mul(b) => self.pair("mul", b, args, nodes),
            Expr::Div(b) => self.pair("div", b, args, nodes),
            Expr::Min(b) => self.pair("min", b, args, nodes),
            Expr::Max(b) => self.pair("max", b, args, nodes),
            Expr::Clamp(b) => self.key("clamp", |c| {
                c.idx(0, |c| c.expr_node(&b.0, args, nodes));
                c.idx(1, |c| c.expr_node(&b.1, args, nodes));
                c.idx(2, |c| c.expr_node(&b.2, args, nodes));
            }),
        }
    }

    fn pair(&mut self, name: &'static str, b: &(Expr, Expr), args: u8, nodes: &mut u32) {
        self.key(name, |c| {
            c.idx(0, |c| c.expr_node(&b.0, args, nodes));
            c.idx(1, |c| c.expr_node(&b.1, args, nodes));
        });
    }
}

#[cfg(test)]
mod advance_tests {
    //! `AttackPhase.advance`, the slide of the main character's combo step (ADR-CONT-3, ADR-SVC-1).

    use serde_json::{Value, json};

    use crate::fixed::Fx;
    use crate::limits::MAX_ADVANCE_DISTANCE;
    use crate::stage::Stage;
    use crate::validate::{Error, validate};

    const STAGE: &str = include_str!("../../tests/fixtures/stage_v0.json");

    fn phase(advance: Value) -> Value {
        json!({ "kind": "active", "ticks": 6, "advance": advance })
    }

    /// The fixture at `sim_version`, its Main weapon's one combo step holding `phase`.
    fn on_weapon(version: u32, phase: Value) -> Value {
        let mut v: Value = serde_json::from_str(STAGE).unwrap();
        v["sim_version"] = json!(version);
        v["player"]["weapon"] = json!({ "inline": {
            "combo": { "main": { "attack": { "inline": [phase] } } },
            "window_ticks": 20, "assist_range": 0 } });
        v
    }

    fn on_enemy(version: u32, phase: Value) -> Value {
        let mut v: Value = serde_json::from_str(STAGE).unwrap();
        v["sim_version"] = json!(version);
        v["waves"][0]["enemy"]["rules"][0]["do"]["attack"] = json!({ "inline": [phase] });
        v
    }

    fn on_boss(version: u32, phase: Value) -> Value {
        let mut v: Value = serde_json::from_str(STAGE).unwrap();
        v["sim_version"] = json!(version);
        v["boss"]["phases"][0]["timeline"][1]["attack"] = json!({ "inline": [phase] });
        v
    }

    fn paths(v: &Value) -> Vec<String> {
        let stage: Stage = serde_json::from_value(v.clone()).expect("the stage parses");
        match validate(&stage) {
            Ok(()) => Vec::new(),
            Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
            Err(other) => panic!("unexpected {other:?}"),
        }
    }

    const AT: &str = "$.player.weapon.inline.combo.main.attack.inline[0].advance";

    #[test]
    fn an_advance_of_one_pixel_to_the_cap_loads_on_a_weapon_step() {
        for stored in [Fx::px(1).0, Fx::px(MAX_ADVANCE_DISTANCE).0] {
            assert!(
                paths(&on_weapon(2, phase(json!(stored)))).is_empty(),
                "{stored}"
            );
        }
    }

    #[test]
    fn an_advance_outside_its_bounds_is_refused_at_its_path() {
        for stored in [0, 255, Fx::px(MAX_ADVANCE_DISTANCE).0 + 1] {
            assert_eq!(paths(&on_weapon(2, phase(json!(stored)))), [AT], "{stored}");
        }
    }

    #[test]
    fn an_enemy_or_boss_attack_never_advances() {
        let enemy = paths(&on_enemy(2, phase(json!(Fx::px(10).0))));
        assert_eq!(
            enemy,
            ["$.waves[0].enemy.rules[0].do.attack.inline[0].advance"]
        );
        let boss = paths(&on_boss(2, phase(json!(Fx::px(10).0))));
        assert_eq!(boss.len(), 1, "{boss:?}");
        assert!(boss[0].ends_with(".advance"), "{boss:?}");
    }

    #[test]
    fn an_advance_needs_sim_version_2() {
        assert_eq!(paths(&on_weapon(1, phase(json!(Fx::px(10).0)))), [AT]);
    }
}
