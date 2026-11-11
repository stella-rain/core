//! The behaviour checks: agents, movement, rules, conditions, selectors, parts and skills.

use super::limits::*;
use super::{Ctx, Foe};
use crate::behaviour::{
    Action, Agent, BossCall, Condition, Elite, Guardians, Movement, Rule, Selector, Skill,
};
use crate::id::ContentId;
use crate::registry::Kind;

impl<'a> Ctx<'a> {
    pub(super) fn agent(&mut self, a: &Agent, summon_ok: bool) {
        self.content_id("base", &a.base, Kind::Asset);
        if let Some(p) = &a.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.within("hp", i64::from(a.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("defence", i64::from(a.defence), 0, i64::from(MAX_DEFENCE));
        if let Some(p) = &a.profile {
            self.key("profile", |c| c.profile(p));
        }
        self.within("radius", i64::from(a.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("movement", |c| {
            c.movement(&a.movement);
            match (&a.movement, &a.elite) {
                (Movement::FleeWhenDisturbed { .. }, None) => {
                    c.fail("flee_when_disturbed needs an elite")
                }
                (Movement::Shrine { .. }, None) => c.fail("shrine needs an elite"),
                (
                    Movement::Shrine { .. } | Movement::FleeWhenDisturbed { .. },
                    Some(Elite { mimic: Some(_), .. }),
                ) => c.fail("a mimic is neither a shrine nor a goblin"),
                _ => {}
            }
        });
        let was = self.ranged_ok;
        self.ranged_ok = self.foe == Foe::Enemy && a.elite.is_none();
        self.rules("rules", &a.rules, summon_ok);
        self.ranged_ok = was;
        if let Some(elite) = &a.elite {
            self.key("elite", |c| {
                c.within("range", i64::from(elite.range.0), 1, i64::from(MAX_LEN));
                if matches!(a.movement, Movement::Shrine { .. })
                    && elite.boss_call != BossCall::Stay
                {
                    c.key("boss_call", |c| c.fail("a shrine's boss_call is stay"));
                } else if elite.mimic.is_some() && elite.boss_call != BossCall::Join {
                    c.key("boss_call", |c| c.fail("a mimic's boss_call is join"));
                }
                if let Some(mimic) = &elite.mimic {
                    c.key("mimic", |c| {
                        c.content_id("disguise", &mimic.disguise, Kind::Asset);
                        c.within(
                            "reveal_radius",
                            i64::from(mimic.reveal_radius.0),
                            1,
                            i64::from(MAX_LEN),
                        );
                    });
                }
                if let Some(aura) = &elite.aura {
                    if elite.mimic.is_some() {
                        c.key("aura", |c| c.fail("a mimic has no aura"));
                    }
                    c.key("aura", |c| {
                        c.within("radius", i64::from(aura.radius.0), 1, i64::from(MAX_LEN));
                        c.within(
                            "damage_percent",
                            i64::from(aura.damage_percent),
                            1,
                            i64::from(MAX_AURA_PERCENT),
                        );
                    });
                }
                if let Some(escort) = &elite.escort {
                    if elite.mimic.is_some() {
                        c.key("escort", |c| c.fail("a mimic has no escort"));
                    }
                    c.key("escort", |c| c.guardians(escort));
                }
            });
        }
    }

    /// A shrine's guardians or a pack leader's escort: a bounded count and ring of ordinary
    /// enemies.
    fn guardians(&mut self, g: &Guardians) {
        self.within(
            "count",
            i64::from(g.count),
            1,
            i64::from(MAX_SHRINE_GUARDIANS),
        );
        self.within("radius", i64::from(g.radius.0), 1, i64::from(MAX_LEN));
        let foe = self.regular();
        self.key("enemy", |c| {
            c.as_foe(foe, |c| c.agent(&g.enemy, false));
            if g.enemy.elite.is_some() {
                c.key("elite", |c| c.fail("a guardian is an ordinary enemy"));
            }
        });
    }

    fn movement(&mut self, m: &Movement) {
        match m {
            Movement::Orbit { radius } => {
                self.within("radius", i64::from(radius.0), 1, i64::from(MAX_LEN))
            }
            Movement::Follow { distance, speed } => {
                self.within("distance", i64::from(distance.0), 0, i64::from(MAX_LEN));
                if let Some(speed) = speed {
                    self.within("speed", i64::from(speed.0), 1, i64::from(MAX_LEN));
                }
            }
            Movement::FleeWhenDisturbed {
                disturb_radius,
                speed,
                leave_after_ticks,
            } => {
                let max = i64::from(MAX_LEN);
                self.within("disturb_radius", i64::from(disturb_radius.0), 1, max);
                self.within("speed", i64::from(speed.0), 1, max);
                self.ticks("leave_after_ticks", *leave_after_ticks, 1);
            }
            Movement::Shrine {
                fight_ticks,
                guardians,
            } => {
                self.ticks("fight_ticks", *fight_ticks, 1);
                self.key("guardians", |c| c.guardians(guardians));
            }
            Movement::Hold { at } => self.point("at", at),
            Movement::Straight { vx, vy } => {
                let max = i64::from(MAX_LEN);
                self.within("vx", i64::from(vx.0), -max, max);
                self.within("vy", i64::from(vy.0), -max, max);
            }
        }
    }

    pub(super) fn rules(&mut self, key: &'static str, rules: &[Rule], summon_ok: bool) {
        if self.at_most(key, rules.len(), MAX_RULES, "rules") {
            self.key(key, |c| {
                for (i, r) in rules.iter().enumerate() {
                    c.idx(i, |c| c.rule(r, summon_ok));
                }
            });
        }
    }

    fn rule(&mut self, r: &Rule, summon_ok: bool) {
        self.key("when", |c| c.condition(&r.when));
        self.key("target", |c| c.selector(&r.target));
        self.key("do", |c| c.action(&r.action, summon_ok));
        self.ticks("cooldown_ticks", r.cooldown_ticks, 0);
        self.charges(r.charges);
    }

    pub(super) fn condition(&mut self, cond: &Condition) {
        match cond {
            Condition::Always => {}
            Condition::HpBelow { who, pct } => {
                self.key("who", |c| c.selector(who));
                self.within("pct", i64::from(*pct), 0, 100);
            }
            Condition::HasStatus { who, status } => {
                self.key("who", |c| c.selector(who));
                self.content_id("status", status, Kind::Status);
            }
            Condition::AggroAbove { who, value } => {
                self.key("who", |c| c.selector(who));
                self.within("value", i64::from(*value), 0, i64::from(MAX_AGGRO));
            }
            Condition::PartBroken { part } => self.part("part", part),
            Condition::TimeAbove { ticks } => self.ticks("ticks", *ticks, 0),
            Condition::NearTarget { distance } => {
                self.within("distance", i64::from(distance.0), 1, i64::from(MAX_LEN))
            }
        }
    }

    fn selector(&mut self, s: &Selector) {
        if let Selector::Part(id) = s {
            self.part("part", id);
        }
    }

    /// A boss part the stage defines.
    fn part(&mut self, key: &'static str, id: &ContentId) {
        if !self.parts.contains(&id.as_str()) {
            self.key(key, |c| c.fail("the boss has no part with this ID"));
        }
    }

    fn action(&mut self, a: &Action, summon_ok: bool) {
        match a {
            Action::Cast { skill } => self.key("skill", |c| c.skill(skill)),
            Action::Attack { attack, ranged } => {
                if *ranged && !self.ranged_ok {
                    self.key("ranged", |c| {
                        c.fail("only a regular enemy's rule is ranged, not a companion's, an elite's or the boss's")
                    });
                }
                self.key("attack", |c| c.attack_ref(attack))
            }
            Action::Summon { agent, count } => {
                if !summon_ok {
                    self.fail("a summoned agent cannot summon");
                    return;
                }
                self.within("count", i64::from(*count), 1, i64::from(MAX_SUMMON_COUNT));
                let foe = self.regular();
                self.key("agent", |c| c.as_foe(foe, |c| c.agent(agent, false)));
            }
            Action::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Action::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
        }
    }

    fn skill(&mut self, s: &Skill) {
        match s {
            Skill::Shot { pattern, damage } => {
                self.key("pattern", |c| c.attack_ref(pattern));
                self.within("damage", i64::from(*damage), 1, i64::from(MAX_DAMAGE));
            }
            Skill::AtkUp {
                value_pct,
                duration_ticks,
            }
            | Skill::Vulnerable {
                value_pct,
                duration_ticks,
            } => {
                self.within(
                    "value_pct",
                    i64::from(*value_pct),
                    1,
                    i64::from(MAX_BUFF_PCT),
                );
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Slow {
                value_pct,
                duration_ticks,
            } => {
                self.within("value_pct", i64::from(*value_pct), 1, 100);
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Heal { amount } => {
                self.within("amount", i64::from(*amount), 1, i64::from(MAX_DAMAGE))
            }
        }
    }
}
