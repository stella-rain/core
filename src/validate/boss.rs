//! The boss checks: foes, boss parts, profiles, phases and steps.

use super::limits::*;
use super::{Ctx, Foe};
use crate::attack::{AttackRef, DamageType, Profile};
use crate::registry::{self, Kind};
use crate::stage::{Boss, BossPart, Phase, Step, Transition};

impl<'a> Ctx<'a> {
    /// Runs `f` as the attacks of `foe`.
    pub(super) fn as_foe(&mut self, foe: Foe, f: impl FnOnce(&mut Self)) {
        let was = std::mem::replace(&mut self.foe, foe);
        f(self);
        self.foe = was;
    }

    /// What an agent summoned or placed by the current one is: a regular enemy, unless the
    /// current one is an ally's.
    pub(super) fn regular(&self) -> Foe {
        if self.foe == Foe::Other {
            Foe::Other
        } else {
            Foe::Enemy
        }
    }

    pub(super) fn boss(&mut self, b: &Boss) {
        self.as_foe(Foe::Boss, |c| c.boss_inner(b));
    }

    fn boss_inner(&mut self, b: &Boss) {
        self.content_id("base", &b.base, Kind::Asset);
        if let Some(p) = &b.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.threat_colour(&b.threat_colour);
        self.stagger(b.stagger_max, b.staggered_ticks, true);
        self.within("hp", i64::from(b.hp), 1, i64::from(MAX_BOSS_HP));
        if let Some(base) = b.stardust {
            self.within("stardust", i64::from(base), 0, i64::from(MAX_STARDUST_BASE));
        }
        self.within("defence", i64::from(b.defence), 0, i64::from(MAX_DEFENCE));
        if let Some(p) = &b.profile {
            self.key("profile", |c| c.profile(p));
        }
        self.within("radius", i64::from(b.radius.0), 1, i64::from(MAX_RADIUS));
        self.point("spawn", &b.spawn);
        if self.at_most("parts", b.parts.len(), MAX_BOSS_PARTS, "boss parts") {
            self.key("parts", |c| {
                for (i, p) in b.parts.iter().enumerate() {
                    let repeated = b.parts[..i].iter().any(|q| q.id == p.id);
                    c.idx(i, |c| c.boss_part(p, repeated));
                }
            });
        }
        if let Some(points) = &b.expertise {
            self.expertise_points(points);
        }
        if let Some(chase) = &b.chase {
            self.key("chase", |c| {
                if c.sim_version < 2 {
                    c.fail("needs sim_version 2");
                    return;
                }
                c.within(
                    "speed",
                    i64::from(chase.speed.0),
                    1,
                    i64::from(MAX_CHASE_SPEED),
                );
                let longer = c.map.width.0.max(c.map.height.0);
                c.within(
                    "stop_distance",
                    i64::from(chase.stop_distance.0),
                    1,
                    i64::from(longer),
                );
            });
        }
        if b.arrives_at_tick.is_some() && self.sim_version < 2 {
            self.key("arrives_at_tick", |c| c.fail("needs sim_version 2"));
        }
        if let Some(call) = &b.call_elites {
            self.key("call_elites", |c| {
                if call.after_ticks.is_none() && call.below_pct.is_none() {
                    c.fail("needs `after_ticks`, `below_pct` or both");
                }
                if let Some(t) = call.after_ticks {
                    c.ticks("after_ticks", t, 0);
                }
                if let Some(pct) = call.below_pct {
                    c.within("below_pct", i64::from(pct), 1, 100);
                }
            });
        }
        if b.phases.is_empty() {
            self.key("phases", |c| c.fail("the boss needs at least one phase"));
        } else if self.at_most("phases", b.phases.len(), MAX_PHASES, "phases") {
            self.key("phases", |c| {
                for (i, p) in b.phases.iter().enumerate() {
                    c.idx(i, |c| c.phase(p, i + 1 == b.phases.len()));
                }
            });
        }
    }

    fn boss_part(&mut self, p: &BossPart, repeated: bool) {
        self.key("id", |c| {
            if !registry::is_well_formed(p.id.as_str()) {
                c.fail("not a valid part ID (lowercase snake_case, at most 32 characters)");
            } else if repeated {
                c.fail("two parts have this ID");
            }
        });
        self.content_id("asset", &p.asset, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("defence", i64::from(p.defence), 0, i64::from(MAX_DEFENCE));
        self.within("radius", i64::from(p.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("offset", |c| {
            let max = i64::from(MAX_LEN);
            c.within("x", i64::from(p.offset.x.0), -max, max);
            c.within("y", i64::from(p.offset.y.0), -max, max);
        });
        if let Some(profile) = &p.profile {
            self.key("profile", |c| c.profile(profile));
        }
    }

    /// A match-up profile: each type at most once in each list, and never in
    /// both. The one blanket resist is the parser's: `resist` holds a single value.
    pub(super) fn profile(&mut self, p: &Profile) {
        let types = DamageType::ALL.len();
        for (key, list) in [("weak", &p.weak), ("strong", &p.strong)] {
            if !self.at_most(key, list.len(), types, "types") {
                continue;
            }
            self.key(key, |c| {
                for (i, ty) in list.iter().enumerate() {
                    if list[..i].contains(ty) {
                        c.idx(i, |c| c.fail("this type is already in the list"));
                    } else if key == "strong" && p.weak.contains(ty) {
                        c.idx(i, |c| c.fail("this type is also in `weak`"));
                    }
                }
            });
        }
    }

    fn phase(&mut self, p: &Phase, last: bool) {
        match (&p.until, last) {
            (Some(cond), false) => self.key("until", |c| c.condition(cond)),
            (None, false) => self.key("until", |c| c.fail("every phase but the last needs one")),
            (Some(_), true) => self.key("until", |c| {
                c.fail("the last phase lasts until the boss dies")
            }),
            (None, true) => {}
        }
        if self.at_most(
            "transition",
            p.transition.len(),
            MAX_TRANSITIONS,
            "transitions",
        ) {
            self.key("transition", |c| {
                for (i, t) in p.transition.iter().enumerate() {
                    if let Transition::InvulnerableTicks { ticks } = t {
                        c.idx(i, |c| c.ticks("ticks", *ticks, 1));
                    }
                }
            });
        }
        if self.at_most(
            "timeline",
            p.timeline.len(),
            MAX_TIMELINE_STEPS,
            "timeline steps",
        ) {
            self.key("timeline", |c| {
                for (i, s) in p.timeline.iter().enumerate() {
                    c.idx(i, |c| c.step(s, i + 1 == p.timeline.len()));
                }
            });
        }
        self.rules("rules", &p.rules, true);
    }

    fn step(&mut self, s: &Step, last: bool) {
        match s {
            Step::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Step::Wait { ticks } | Step::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
            Step::Attack { attack, big } => {
                self.key("attack", |c| c.attack_ref(attack));
                if *big && !matches!(attack, AttackRef::Preset { .. }) {
                    self.key("big", |c| {
                        c.fail("only a preset can be big: its ID names it")
                    });
                }
            }
            Step::Loop => {
                if !last {
                    self.fail("`loop` must be the last step; later steps would never run");
                }
            }
        }
    }
}
