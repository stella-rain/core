//! The stage, player, wave and star checks.

use super::limits::*;
use super::{Ctx, Foe, wide};
use crate::registry::Kind;
use crate::rewards::{MAX_WEIGHT, Rarity};
use crate::stage::{
    ExpertisePoints, Healing, Levels, Player, Side, Stage, Stars, Tier, Wave,
    stardust_tiers_problem,
};

impl<'a> Ctx<'a> {
    pub(super) fn stage(&mut self, s: &Stage) {
        if let Some(map) = &s.map {
            self.key("map", |c| c.map(map));
        }
        self.key("id", |c| {
            let ok = (1..=MAX_STAGE_ID_CHARS).contains(&s.id.len())
                && s.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !s.id.starts_with('-')
                && !s.id.ends_with('-');
            if !ok {
                c.fail(format!(
                    "a stage ID is 1 to {MAX_STAGE_ID_CHARS} characters of a-z, 0-9 and -, \
                     not starting or ending with -"
                ));
            }
        });
        let title = s.title.chars().count();
        if title == 0 {
            self.key("title", |c| c.fail("must not be empty"));
        } else {
            self.within("title", wide(title), 1, wide(MAX_TITLE_CHARS));
        }
        self.within(
            "description",
            wide(s.description.chars().count()),
            0,
            wide(MAX_DESCRIPTION_CHARS),
        );
        self.ticks("length_ticks", s.length_ticks, 1);
        self.key("player", |c| c.player(&s.player));
        if self.at_most(
            "companions",
            s.companions.len(),
            MAX_COMPANIONS,
            "companions",
        ) {
            self.key("companions", |c| {
                for (i, a) in s.companions.iter().enumerate() {
                    c.idx(i, |c| {
                        c.agent(a, true);
                        if a.elite.is_some() {
                            c.key("elite", |c| c.fail("a companion cannot be an elite"));
                        }
                        if a.profile.is_some() {
                            c.key("profile", |c| c.fail("a companion has no match-up profile"));
                        }
                    });
                }
            });
        }
        if self.at_most("waves", s.waves.len(), MAX_WAVES, "waves") {
            self.key("waves", |c| {
                for (i, w) in s.waves.iter().enumerate() {
                    c.idx(i, |c| c.wave(w, s.length_ticks, MAX_WAVE_COUNT));
                }
            });
        }
        if let Some(boss) = &s.boss {
            self.key("boss", |c| c.boss(boss));
            if let Some(at) = boss.arrives_at_tick.filter(|_| self.sim_version >= 2) {
                self.key("boss", |c| {
                    c.within(
                        "arrives_at_tick",
                        i64::from(at),
                        1,
                        i64::from(s.length_ticks) - 1,
                    )
                });
            }
        }
        if let Some(levels) = &s.levels {
            self.key("levels", |c| c.levels(levels));
        }
        if let Some(table) = s.level_table
            && let Some(problem) = table.problem()
        {
            self.key("level_table", |c| c.fail(problem));
        }
        if let Some(problem) = s
            .stardust
            .as_ref()
            .and_then(|f| stardust_tiers_problem(&f.tiers))
        {
            self.key("stardust", |c| c.key("tiers", |c| c.fail(problem)));
        }
        if let Some(base) = s.stardust.as_ref().and_then(|f| f.summon_base) {
            self.key("stardust", |c| {
                c.within(
                    "summon_base",
                    i64::from(base),
                    0,
                    i64::from(MAX_STARDUST_BASE),
                );
            });
        }
        if let Some(target) = s.crowd_target {
            self.within(
                "crowd_target",
                i64::from(target),
                1,
                i64::from(MAX_CROWD_TARGET),
            );
        }
        if let Some(cap) = s.elite_cap {
            self.within("elite_cap", i64::from(cap), 1, i64::from(MAX_ELITES_ALIVE));
        }
        if let Some(stars) = &s.stars {
            self.key("stars", |c| {
                if s.levels.is_none() {
                    c.fail("needs `levels`: a star's rarity follows the player's level".to_owned());
                }
                c.stars(stars);
            });
        }
        if let Some(heal) = &s.heal {
            self.key("heal", |c| c.heal(heal));
        }
    }

    fn heal(&mut self, h: &Healing) {
        self.within(
            "boss_pct",
            i64::from(h.boss_pct),
            0,
            i64::from(MAX_HEAL_PCT),
        );
        self.within(
            "drop_one_in",
            i64::from(h.drop_one_in),
            1,
            i64::from(MAX_HEAL_ONE_IN),
        );
        self.within("amount", i64::from(h.amount), 1, i64::from(MAX_HEAL_AMOUNT));
        self.within("cap", i64::from(h.cap), 1, i64::from(MAX_HEAL_PICKUPS));
        self.within(
            "pickup_radius",
            i64::from(h.pickup_radius.0),
            1,
            i64::from(MAX_PICKUP_RADIUS),
        );
    }

    pub(super) fn stars(&mut self, st: &Stars) {
        self.within(
            "drop_per_mille",
            i64::from(st.drop_per_mille),
            0,
            i64::from(MAX_DROP_PER_MILLE),
        );
        self.within("pity", i64::from(st.pity), 1, i64::from(MAX_PITY));
        self.within("cap", i64::from(st.cap), 1, i64::from(MAX_STARS_ON_MAP));
        self.within(
            "pickup_radius",
            i64::from(st.pickup_radius.0),
            1,
            i64::from(MAX_PICKUP_RADIUS),
        );
        self.within("rerolls", i64::from(st.rerolls), 0, i64::from(MAX_REROLLS));
        self.within(
            "banishes",
            i64::from(st.banishes),
            0,
            i64::from(MAX_BANISHES),
        );
        if st.tiers.is_empty() {
            self.key("tiers", |c| c.fail("needs at least one tier".to_owned()));
        } else if self.at_most("tiers", st.tiers.len(), MAX_TIERS, "tiers") {
            self.key("tiers", |c| {
                let mut previous = 0;
                for (i, tier) in st.tiers.iter().enumerate() {
                    c.idx(i, |c| c.tier(tier, i, previous));
                    previous = tier.from_level;
                }
            });
        }
    }

    /// Tier `index` of a drop table, after one that starts at level `previous`.
    fn tier(&mut self, tier: &Tier, index: usize, previous: u32) {
        let level = tier.from_level;
        if index == 0 && level != 1 {
            self.key("from_level", |c| {
                c.fail(format!("the first tier starts at level 1 (found {level})"))
            });
        } else if index > 0 && !(previous + 1..=MAX_TIER_LEVEL).contains(&level) {
            self.key("from_level", |c| {
                c.fail(format!(
                    "must be above the tier before ({previous}) and at most {MAX_TIER_LEVEL}                      (found {level})"
                ))
            });
        }
        self.key("weights", |c| {
            for rarity in Rarity::ALL {
                let weight = tier.weights.get(rarity);
                if weight > MAX_RARITY_WEIGHT {
                    c.fail(format!(
                        "{rarity:?} weighs at most {MAX_RARITY_WEIGHT} (found {weight})"
                    ));
                }
            }
            if tier.weights.total() == 0 {
                c.fail("at least one rarity must weigh more than 0".to_owned());
            }
        });
        if let Some(weight) = tier.random_rune_weight {
            self.within(
                "random_rune_weight",
                i64::from(weight),
                1,
                i64::from(MAX_WEIGHT),
            );
        }
    }

    pub(super) fn levels(&mut self, l: &Levels) {
        if l.xp.is_empty() {
            self.key("xp", |c| c.fail("needs at least one level".to_owned()));
        } else if self.at_most("xp", l.xp.len(), MAX_LEVELS, "levels") {
            self.key("xp", |c| {
                for (i, xp) in l.xp.iter().enumerate() {
                    if !(1..=MAX_LEVEL_XP).contains(xp) {
                        c.idx(i, |c| {
                            c.fail(format!("must be between 1 and {MAX_LEVEL_XP} (found {xp})"))
                        });
                    }
                }
            });
        }
    }

    fn player(&mut self, p: &Player) {
        self.content_id("base", &p.base, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_PLAYER_HP));
        self.within("atk", i64::from(p.atk), 1, i64::from(MAX_PLAYER_ATK));
        self.within("defence", i64::from(p.defence), 0, i64::from(MAX_DEFENCE));
        self.key("weapon", |c| c.weapon_ref(&p.weapon, Self::weapon));
        self.key("evade", |c| c.evade(&p.evade));
    }

    pub(super) fn wave(&mut self, w: &Wave, length_ticks: u32, max_count: u16) {
        if w.at_tick >= length_ticks {
            self.key("at_tick", |c| c.fail("must be before the stage ends"));
        }
        self.key("enemy", |c| {
            c.as_foe(Foe::Enemy, |c| c.agent(&w.enemy, true))
        });
        match (&w.spawn, &w.edge) {
            (Some(_), Some(_)) => {
                self.key("edge", |c| c.fail("give `spawn` or `edge`, not both"));
            }
            (None, None) => self.key("spawn", |c| c.fail("give `spawn` or `edge`")),
            (Some(p), None) => self.point("spawn", p),
            (None, Some(edge)) => self.key("edge", |c| {
                if let Some(along) = edge.along {
                    c.within("along", i64::from(along), -1000, 1000);
                    if edge.side == Side::Any {
                        c.key("along", |c| c.fail("not with the side `any`"));
                    }
                }
            }),
        }
        self.within("count", i64::from(w.count), 1, i64::from(max_count));
        self.ticks("every_ticks", w.every_ticks, u32::from(w.count > 1));
        if let Some(xp) = w.xp {
            self.within("xp", i64::from(xp), 0, i64::from(MAX_ENEMY_XP));
        }
        if let Some(points) = &w.expertise {
            self.expertise_points(points);
        }
        if let Some(base) = w.stardust {
            self.within("stardust", i64::from(base), 0, i64::from(MAX_STARDUST_BASE));
        }
    }

    pub(super) fn expertise_points(&mut self, points: &ExpertisePoints) {
        self.key("expertise", |c| {
            c.within(
                "hit",
                i64::from(points.hit),
                0,
                i64::from(MAX_EXPERTISE_PER_HIT),
            );
            c.within(
                "kill",
                i64::from(points.kill),
                0,
                i64::from(MAX_EXPERTISE_PER_KILL),
            );
        });
    }
}
