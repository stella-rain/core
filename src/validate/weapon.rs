//! The weapon checks: references, branches, holds, smashes, charges and the evade.

use super::Ctx;
use super::limits::*;
use crate::attack::{AttackRef, PhaseKind};
use crate::presets;
use crate::registry::{self, Kind};
use crate::stage::Evade;
use crate::weapon::{Branches, Charge, Hold, SmashEffect, Weapon, WeaponRef};

/// What is wrong with a tree ID that is not well formed.
pub(crate) const TREE_ID_PROBLEM: &str =
    "not a valid ID (lowercase snake_case, at most 32 characters)";

impl<'a> Ctx<'a> {
    /// A weapon preset's ID, or an inline weapon checked by `inline`.
    pub(super) fn weapon_ref<W>(&mut self, w: &WeaponRef<W>, inline: impl FnOnce(&mut Self, &W)) {
        match w {
            WeaponRef::Preset { id, tree } => self.key("preset", |c| {
                c.content_id("id", id, Kind::Weapon);
                // Whether the weapon has that tree is the weapons file's to say: the run checks
                // it against its own content (`World::new`).
                if tree
                    .as_ref()
                    .is_some_and(|t| !registry::is_well_formed(t.as_str()))
                {
                    c.key("tree", |c| c.fail(TREE_ID_PROBLEM));
                }
            }),
            WeaponRef::Inline(w) => self.key("inline", |c| inline(c, w)),
        }
    }

    /// The weapon's combo tree: at most `MAX_COMBO_STEPS` steps in all, none deeper
    /// than `MAX_COMBO_DEPTH`, each attack and charge checked, and its smash effect.
    pub(super) fn weapon(&mut self, w: &Weapon) {
        let mut steps = 0;
        self.key("combo", |c| c.branches(&w.combo, 1, &mut steps));
        self.key("dash", |c| c.branches(&w.dash, 1, &mut steps));
        if steps > MAX_COMBO_STEPS {
            self.key("combo", |c| {
                c.fail(format!(
                    "at most {MAX_COMBO_STEPS} steps in a weapon's combos"
                ))
            });
        }
        self.within(
            "window_ticks",
            i64::from(w.window_ticks),
            0,
            i64::from(MAX_COMBO_WINDOW_TICKS),
        );
        self.within(
            "dash_strike_ticks",
            i64::from(w.dash_strike_ticks),
            0,
            i64::from(MAX_DASH_STRIKE_TICKS),
        );
        self.within(
            "assist_range",
            i64::from(w.assist_range.0),
            0,
            i64::from(MAX_ASSIST_RANGE),
        );
        if let Some(effect) = &w.smash {
            self.key("smash", |c| c.smash(effect));
        }
    }

    fn branches(&mut self, b: &Branches, depth: u32, steps: &mut usize) {
        for (key, step) in [("main", &b.main), ("smash", &b.smash)] {
            let Some(step) = step else {
                continue;
            };
            *steps += 1;
            self.key(key, |c| {
                if depth > MAX_COMBO_DEPTH {
                    c.fail(format!("a combo is at most {MAX_COMBO_DEPTH} steps deep"));
                    return;
                }
                c.key("attack", |c| c.attack_ref(&step.attack));
                if let Some(charge) = &step.charge {
                    c.key("charge", |c| c.charge(charge));
                }
                if let Some(hold) = &step.hold {
                    if step.charge.is_some() {
                        c.key("hold", |c| {
                            c.fail("a step has a charge or a hold, never both")
                        });
                    }
                    c.key("hold", |c| c.hold(hold));
                    // A preset's phases are the content's: `presets::hold_problems` checks them
                    // where the content is known (CS1b-1).
                    if matches!(&step.attack, AttackRef::Inline(p) if !presets::holds_after_active(p))
                    {
                        c.key("attack", |c| {
                            c.fail(
                                "a held step's attack has a recovery with no spawn after its last active phase, where the hold begins",
                            )
                        });
                    }
                }
                c.key("next", |c| c.branches(&step.next, depth + 1, steps));
            });
        }
    }

    /// A held step's hold: the ticks held and the movement are bounded, the
    /// loop's phases are all `active`, and both attacks are checked as any attack's are.
    fn hold(&mut self, h: &Hold) {
        self.within(
            "max_ticks",
            i64::from(h.max_ticks),
            1,
            i64::from(MAX_HOLD_TICKS),
        );
        self.within("move_pct", i64::from(h.move_pct), 0, 100);
        self.key("loop", |c| {
            c.attack_ref(&h.loop_attack);
            if matches!(&h.loop_attack, AttackRef::Inline(p) if p.iter().any(|q| q.kind != PhaseKind::Active))
            {
                c.fail("a loop's phases are all active");
            }
        });
        self.key("release", |c| c.attack_ref(&h.release));
    }

    fn smash(&mut self, effect: &SmashEffect) {
        match *effect {
            SmashEffect::Pull { distance, ticks } | SmashEffect::Knockback { distance, ticks } => {
                self.within(
                    "distance",
                    i64::from(distance.0),
                    0,
                    i64::from(MAX_PULL_DISTANCE),
                );
                self.within("ticks", i64::from(ticks), 1, i64::from(MAX_PULL_TICKS));
            }
            // No numbers of its own: the rune's come from the runes file, bounded there.
            SmashEffect::Bleed {} => {}
            SmashEffect::Stun { ticks } => {
                self.within("ticks", i64::from(ticks), 1, i64::from(MAX_STUN_TICKS));
            }
        }
    }

    /// A charged step. Level 0 is the step's own attack; the levels here are 1 and
    /// up, after 0, rising strictly and all before `max_ticks`; a perfect window ends by the next
    /// level (or `max_ticks`).
    fn charge(&mut self, c: &Charge) {
        self.within(
            "max_ticks",
            i64::from(c.max_ticks),
            1,
            i64::from(MAX_CHARGE_TICKS),
        );
        self.within("move_pct", i64::from(c.move_pct), 0, 100);
        if c.levels.is_empty() {
            self.key("levels", |c| c.fail("a charge has at least one level"));
            return;
        }
        if !self.at_most("levels", c.levels.len(), MAX_CHARGE_LEVELS, "levels") {
            return;
        }
        self.key("levels", |ctx| {
            for (i, l) in c.levels.iter().enumerate() {
                let next = c.levels.get(i + 1).map_or(c.max_ticks, |n| n.at_ticks);
                ctx.idx(i, |ctx| {
                    if i == 0 && l.at_ticks == 0 {
                        ctx.key("at_ticks", |c| {
                            c.fail("level 1 comes after 0 (level 0 is the step's own attack)")
                        });
                    }
                    if i > 0 && l.at_ticks <= c.levels[i - 1].at_ticks {
                        ctx.key("at_ticks", |c| c.fail("levels rise strictly"));
                    }
                    if l.at_ticks >= c.max_ticks {
                        ctx.key("at_ticks", |c| c.fail("a level comes before max_ticks"));
                    }
                    if u64::from(l.at_ticks) + u64::from(l.perfect_ticks) > u64::from(next) {
                        ctx.key("perfect_ticks", |c| {
                            c.fail("a perfect window ends by the next level, or max_ticks")
                        });
                    }
                    ctx.within(
                        "perfect_pct",
                        i64::from(l.perfect_pct),
                        0,
                        i64::from(MAX_PERFECT_PCT),
                    );
                    ctx.key("attack", |c| c.attack_ref(&l.attack));
                    if let Some(a) = &l.perfect_attack {
                        ctx.key("perfect_attack", |c| c.attack_ref(a));
                    }
                });
            }
        });
    }

    pub(super) fn evade(&mut self, e: &Evade) {
        self.within(
            "distance",
            i64::from(e.distance.0),
            0,
            i64::from(MAX_EVADE_DISTANCE),
        );
        self.within("ticks", i64::from(e.ticks), 1, i64::from(MAX_EVADE_TICKS));
        self.within(
            "invulnerable_ticks",
            i64::from(e.invulnerable_ticks),
            0,
            i64::from(MAX_EVADE_TICKS),
        );
        self.ticks("cooldown_ticks", e.cooldown_ticks, 0);
        let most = i64::from(MAX_EVADE_CHARGES);
        self.within("charges", i64::from(e.charges), 1, most);
        if let Some(max) = e.max_charges {
            self.within(
                "max_charges",
                i64::from(max),
                i64::from(e.charges).max(1),
                most,
            );
        }
    }

    pub(super) fn charges(&mut self, charges: Option<u16>) {
        if let Some(n) = charges {
            self.within("charges", i64::from(n), 1, i64::from(u16::MAX));
        }
    }
}
