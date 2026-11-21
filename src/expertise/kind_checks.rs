//! The style-kind checks: each closed kind's numbers, and the per-path and per-step rules.

use super::{
    AreaBonus, Breaker, BreakerEffect, BurstEffect, ComboTempo, CounterBonus, CounterEffect,
    Encore, Gauge, GaugeSpend, GiantTrigger, KnockdownSteps, MAX_COUNTED_PRESSES,
    MAX_COUNTER_EFFECTS, MAX_HOLD_MOVE_PCT, MAX_KNOCKDOWN_STEPS, MAX_LEVEL_KINDS, MAX_PROC_PCT,
    MAX_REPEAT_AFTER_TICKS, MAX_RUNE_SURGE_STEPS, MAX_SMASH_REPEATS, MAX_SPIRIT_BLEED_STACKS,
    MAX_TEMPO_EFFECTS, MAX_TRIGGER_EFFECTS, MAX_VULNERABLE_AFTER_TICKS, SmashRepeat, Spirit,
    SpiritEffect, StepRef, Style, StyleKind, TempoEffect, TriggerEffect, style_levels,
};
use crate::event::Press;
use crate::fixed::Fx;
use crate::limits::{
    MAX_COUNTER_WINDOW_TICKS, MAX_ENCORE_TICKS, MAX_GAUGE_DAMAGE_PCT, MAX_PCT, MAX_SIZE_STACKS,
    MAX_SPIRIT_DECAY_TICKS, MAX_SPIRIT_LEVEL, MAX_TEMPO_STACKS, MAX_TRIGGER_TICKS,
    MAX_WEAPON_GAUGE,
};
use crate::registry;
use crate::rewards::{MAX_BURST_RADIUS_PX, MAX_RANGE_PCT, MAX_RUNE_HASTE_PCT};
use crate::run_stats::MAX_ATTACK_SPEED_PCT;
use crate::validate;
use crate::validate::limits::MAX_COMBO_DEPTH;

/// A gauge's numbers, each in its own range, and its full-gauge attack as a player's attack.
pub(super) fn check_gauge(issue: &mut impl FnMut(String, String), at: &str, g: &Gauge) {
    if !(1..=MAX_WEAPON_GAUGE).contains(&g.max) {
        issue(
            format!("{at}.max"),
            format!("1 to {MAX_WEAPON_GAUGE} (found {})", g.max),
        );
    }
    // The fill bounds are read against the gauge's own length; a `max` out of range is its own issue.
    let most = g.max.min(MAX_WEAPON_GAUGE);
    for (key, v) in [("per_step", g.per_step), ("per_hit", g.per_hit)] {
        if v > most {
            issue(
                format!("{at}.{key}"),
                format!("0 to the gauge's max {most} (found {v})"),
            );
        }
    }
    if g.per_rune > most {
        issue(
            format!("{at}.per_rune"),
            format!("0 to the gauge's max {most} (found {})", g.per_rune),
        );
    }
    if g.outer_half_pct > MAX_PCT {
        issue(
            format!("{at}.outer_half_pct"),
            format!("0 to {MAX_PCT} (found {})", g.outer_half_pct),
        );
    } else if g.outer_half_pct != 0 && g.per_step == 0 {
        issue(
            format!("{at}.outer_half_pct"),
            "raises per_step, which is 0".into(),
        );
    }
    // `per_smash_hit` and `smash_cap` are named together, the hit inside the cap, the cap inside
    // the bar; `smash_cap` replaces `step_cap` for smash steps only.
    if g.smash_cap > most {
        issue(
            format!("{at}.smash_cap"),
            format!("0 to the gauge's max {most} (found {})", g.smash_cap),
        );
    }
    if g.per_smash_hit > most {
        issue(
            format!("{at}.per_smash_hit"),
            format!("0 to the gauge's max {most} (found {})", g.per_smash_hit),
        );
    } else if g.per_smash_hit != 0 && g.smash_cap == 0 {
        issue(format!("{at}.smash_cap"), "needs smash_cap".into());
    } else if g.smash_cap != 0 && g.per_smash_hit == 0 {
        issue(format!("{at}.smash_cap"), "needs per_smash_hit".into());
    } else if g.per_smash_hit > g.smash_cap {
        issue(
            format!("{at}.per_smash_hit"),
            format!(
                "at most smash_cap {} (found {})",
                g.smash_cap, g.per_smash_hit
            ),
        );
    }
    if g.from_damage_pct > MAX_GAUGE_DAMAGE_PCT {
        issue(
            format!("{at}.from_damage_pct"),
            format!("0 to {MAX_GAUGE_DAMAGE_PCT} (found {})", g.from_damage_pct),
        );
    }
    if g.step_cap == 0 || g.step_cap > most {
        issue(
            format!("{at}.step_cap"),
            format!("1 to the gauge's max {most} (found {})", g.step_cap),
        );
    }
    let GaugeSpend::FullSmash { attack } = &g.spend;
    if let Err(found) = validate::validate_attack_ref(attack, registry::ENTRIES) {
        for f in found {
            issue(
                format!("{at}.spend.attack{}", f.path.trim_start_matches('$')),
                f.message,
            );
        }
    }
}

/// One slot per `SpiritEffect` variant (`name()` is the slot; a test keeps it equal to the schema's).
pub(super) const SPIRIT_EFFECT_KINDS: usize = 11;

/// A spirit's numbers, each in its own range; a level names an effect once.
pub(super) fn check_spirit(issue: &mut impl FnMut(String, String), at: &str, s: &Spirit) {
    if !(1..=u32::from(MAX_SPIRIT_DECAY_TICKS)).contains(&s.decay_ticks) {
        issue(
            format!("{at}.decay_ticks"),
            format!("1 to {MAX_SPIRIT_DECAY_TICKS} (found {})", s.decay_ticks),
        );
    }
    if s.levels.is_empty() || s.levels.len() > usize::from(MAX_SPIRIT_LEVEL) {
        issue(
            format!("{at}.levels"),
            format!("1 to {MAX_SPIRIT_LEVEL} levels (found {})", s.levels.len()),
        );
    }
    if let Some(r) = s
        .raise_per_smash
        .filter(|r| !(1..=MAX_SPIRIT_LEVEL).contains(r))
    {
        issue(
            format!("{at}.raise_per_smash"),
            format!("1 to {MAX_SPIRIT_LEVEL} (found {r})"),
        );
    }
    for (l, level) in s
        .levels
        .iter()
        .enumerate()
        .take(usize::from(MAX_SPIRIT_LEVEL))
    {
        let mut seen = [false; SPIRIT_EFFECT_KINDS];
        for (e, effect) in level.iter().enumerate() {
            let at = format!("{at}.levels[{l}][{e}]");
            let scalar = match effect {
                SpiritEffect::DamagePct { pct } => Some(("pct", *pct, MAX_PCT)),
                SpiritEffect::RangePct { pct } => Some(("pct", *pct, MAX_RANGE_PCT)),
                SpiritEffect::SmashBleedStacks { stacks } => {
                    Some(("stacks", *stacks, MAX_SPIRIT_BLEED_STACKS))
                }
                SpiritEffect::SizeStackCap { stacks }
                | SpiritEffect::SizeStacksPerHit { stacks } => {
                    Some(("stacks", *stacks, u32::from(MAX_SIZE_STACKS)))
                }
                SpiritEffect::SizeStackTicksPct { pct } => Some(("pct", *pct, MAX_PCT)),
                SpiritEffect::RuneCooldownPct { pct } => Some(("pct", *pct, MAX_RUNE_HASTE_PCT)),
                SpiritEffect::ExtraPresses { presses } => {
                    Some(("presses", *presses, MAX_COUNTED_PRESSES))
                }
                SpiritEffect::NormalPull { distance, ticks } => {
                    let most = validate::limits::MAX_PULL_DISTANCE;
                    if !(Fx::px(1).0..=most).contains(&distance.0) {
                        issue(
                            format!("{at}.distance"),
                            format!("{} to {most} (found {})", Fx::px(1).0, distance.0),
                        );
                    }
                    let most = validate::limits::MAX_PULL_TICKS;
                    if !(1..=most).contains(ticks) {
                        issue(
                            format!("{at}.ticks"),
                            format!("1 to {most} (found {ticks})"),
                        );
                    }
                    None
                }
                SpiritEffect::RuneSurge { steps } => {
                    check_rune_surge(issue, &at, steps);
                    None
                }
                SpiritEffect::SmashRepeat { repeats } => {
                    check_smash_repeats(issue, &at, repeats);
                    None
                }
            };
            if let Some((key, v, most)) = scalar
                && !(1..=most).contains(&v)
            {
                issue(format!("{at}.{key}"), format!("1 to {most} (found {v})"));
            }
            let slot = &mut seen[usize::from(effect.name())];
            if *slot {
                issue(at, "the level already names this effect".into());
            }
            *slot = true;
        }
    }
}

/// A `rune_surge`'s steps: a bounded list of legal steps, none sharing a member with another.
fn check_rune_surge(issue: &mut impl FnMut(String, String), at: &str, steps: &[StepRef]) {
    if steps.len() > MAX_RUNE_SURGE_STEPS {
        issue(
            format!("{at}.steps"),
            format!("0 to {MAX_RUNE_SURGE_STEPS} steps (found {})", steps.len()),
        );
    }
    let mut taken: Vec<Vec<Press>> = Vec::new();
    for (i, step) in steps.iter().enumerate().take(MAX_RUNE_SURGE_STEPS) {
        let at = format!("{at}.steps[{i}]");
        // `check_step_len` names the step's own field; this entry is the step.
        check_step_len(
            &mut |path: String, message| {
                issue(path.trim_end_matches(".step").to_string(), message);
            },
            &at,
            step,
        );
        if claims(&mut taken, step) {
            issue(at, "already listed".into());
        }
    }
}

/// A `smash_repeat`'s entries: a bounded list, each number in its own range.
fn check_smash_repeats(issue: &mut impl FnMut(String, String), at: &str, repeats: &[SmashRepeat]) {
    if repeats.is_empty() || repeats.len() > MAX_SMASH_REPEATS {
        issue(
            format!("{at}.repeats"),
            format!("1 to {MAX_SMASH_REPEATS} repeats (found {})", repeats.len()),
        );
    }
    for (r, repeat) in repeats.iter().enumerate().take(MAX_SMASH_REPEATS) {
        let at = format!("{at}.repeats[{r}]");
        let numbers = [
            ("after_ticks", repeat.after_ticks, MAX_REPEAT_AFTER_TICKS),
            ("repeat_pct", repeat.repeat_pct, MAX_PCT),
        ];
        let levels = repeat.min_charge_level.map(|m| {
            let most = u32::try_from(validate::limits::MAX_CHARGE_LEVELS).unwrap_or(u32::MAX);
            ("min_charge_level", m, most)
        });
        for (key, v, most) in numbers.into_iter().chain(levels) {
            if !(1..=most).contains(&v) {
                issue(format!("{at}.{key}"), format!("1 to {most} (found {v})"));
            }
        }
    }
}

/// A combo tempo's numbers, each in its own range; it names an effect once and, unless an
/// earlier level of the path named the kind (`later`), names `speed_pct` and keeps the stacks'
/// total speed within the attack speed ceiling (a later level's fold is `check_step_once`'s).
pub(super) fn check_combo_tempo(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    later: bool,
) {
    let max = u64::from(MAX_TEMPO_STACKS);
    if !(1..=max).contains(&u64::from(t.max_stacks)) {
        issue(
            format!("{at}.max_stacks"),
            format!("1 to {MAX_TEMPO_STACKS} (found {})", t.max_stacks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > MAX_TEMPO_EFFECTS {
        issue(
            format!("{at}.effects"),
            format!(
                "{least} to {MAX_TEMPO_EFFECTS} effects (found {})",
                t.effects.len()
            ),
        );
    }
    let mut seen = [false; MAX_TEMPO_EFFECTS];
    let mut speed = None;
    for (e, effect) in t.effects.iter().enumerate().take(MAX_TEMPO_EFFECTS) {
        let at = format!("{at}.effects[{e}]");
        if let TempoEffect::SpeedPct { pct } = effect {
            if !(1..=MAX_ATTACK_SPEED_PCT).contains(pct) {
                issue(
                    format!("{at}.pct"),
                    format!("1 to {MAX_ATTACK_SPEED_PCT} (found {pct})"),
                );
            }
            speed.get_or_insert(*pct);
        }
        if let TempoEffect::RangePctAtFull { pct } = effect
            && !(1..=MAX_RANGE_PCT).contains(pct)
        {
            issue(
                format!("{at}.pct"),
                format!("1 to {MAX_RANGE_PCT} (found {pct})"),
            );
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
    if !later && !t.effects.is_empty() && t.effects.len() <= MAX_TEMPO_EFFECTS {
        match speed {
            None => issue(format!("{at}.effects"), "names no speed_pct".into()),
            Some(pct) if (1..=MAX_ATTACK_SPEED_PCT).contains(&pct) => {
                let total = u64::from(t.max_stacks) * u64::from(pct);
                if total > u64::from(MAX_ATTACK_SPEED_PCT) {
                    issue(
                        format!("{at}.effects"),
                        format!(
                            "max_stacks times speed_pct is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
                        ),
                    );
                }
            }
            Some(_) => {}
        }
    }
}

/// A giant trigger's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
pub(super) fn check_giant_trigger(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &GiantTrigger,
    later: bool,
) {
    if !(1..=u32::from(MAX_TRIGGER_TICKS)).contains(&t.ticks) {
        issue(
            format!("{at}.ticks"),
            format!("1 to {MAX_TRIGGER_TICKS} (found {})", t.ticks),
        );
    }
    let least = usize::from(!later);
    if t.effects.len() < least || t.effects.len() > MAX_TRIGGER_EFFECTS {
        issue(
            format!("{at}.effects"),
            format!(
                "{least} to {MAX_TRIGGER_EFFECTS} effects (found {})",
                t.effects.len()
            ),
        );
    }
    let mut seen = [false; MAX_TRIGGER_EFFECTS];
    for (e, effect) in t.effects.iter().enumerate().take(MAX_TRIGGER_EFFECTS) {
        let at = format!("{at}.effects[{e}]");
        let bound = match effect {
            TriggerEffect::RangePct { pct } => Some((*pct, 1, MAX_RANGE_PCT)),
            TriggerEffect::SmashPct { pct } => Some((*pct, 1, MAX_PCT)),
            TriggerEffect::RallyPct { pct } => Some((*pct, 101, MAX_PCT)),
            TriggerEffect::HoldMovePct { pct } => Some((*pct, 1, MAX_HOLD_MOVE_PCT)),
            TriggerEffect::SuperArmour {} | TriggerEffect::EndRing {} => None,
        };
        if let Some((pct, lo, hi)) = bound
            && !(lo..=hi).contains(&pct)
        {
            issue(format!("{at}.pct"), format!("{lo} to {hi} (found {pct})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A breaker's numbers, each in its own range; it names an effect once, its burst too.
pub(super) fn check_breaker(issue: &mut impl FnMut(String, String), at: &str, b: &Breaker) {
    if b.effects.is_empty() || b.effects.len() > 5 {
        issue(
            format!("{at}.effects"),
            format!("1 to 5 effects (found {})", b.effects.len()),
        );
    }
    let mut seen = [false; 5];
    for (e, effect) in b.effects.iter().enumerate().take(5) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BreakerEffect::StaggerPct { pct }
            | BreakerEffect::PoisePct { pct }
            | BreakerEffect::PartBreakPct { pct }
            | BreakerEffect::StaggerTicksPct { pct } => {
                if !(101..=MAX_PCT).contains(pct) {
                    issue(
                        format!("{at}.pct"),
                        format!("101 to {MAX_PCT} (found {pct})"),
                    );
                }
            }
            BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                if !(1..=MAX_BURST_RADIUS_PX).contains(radius_px) {
                    issue(
                        format!("{at}.radius_px"),
                        format!("1 to {MAX_BURST_RADIUS_PX} (found {radius_px})"),
                    );
                }
                check_burst(issue, &at, effects);
            }
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

/// A burst's effects: 1 to 3, each in its own range, each variant once.
fn check_burst(issue: &mut impl FnMut(String, String), at: &str, effects: &[BurstEffect]) {
    if effects.is_empty() || effects.len() > 3 {
        issue(
            format!("{at}.effects"),
            format!("1 to 3 effects (found {})", effects.len()),
        );
    }
    let mut seen = [false; 3];
    for (e, effect) in effects.iter().enumerate().take(3) {
        let at = format!("{at}.effects[{e}]");
        match effect {
            BurstEffect::DamagePct { pct } => {
                if !(1..=MAX_PCT).contains(pct) {
                    issue(format!("{at}.pct"), format!("1 to {MAX_PCT} (found {pct})"));
                }
            }
            BurstEffect::StunTicks { ticks } => {
                let max = validate::limits::MAX_STUN_TICKS;
                if !(1..=max).contains(ticks) {
                    issue(format!("{at}.ticks"), format!("1 to {max} (found {ticks})"));
                }
            }
            BurstEffect::KnockDown {} => {}
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the burst already names this effect".into());
        }
        *slot = true;
    }
}

/// A counter bonus's numbers, each in its own range; it names an effect once, and names at
/// least one unless an earlier level of the path named the kind (`later`).
pub(super) fn check_counter_bonus(
    issue: &mut impl FnMut(String, String),
    at: &str,
    c: &CounterBonus,
    later: bool,
) {
    if !(1..=u32::from(MAX_COUNTER_WINDOW_TICKS)).contains(&c.window_ticks) {
        issue(
            format!("{at}.window_ticks"),
            format!("1 to {MAX_COUNTER_WINDOW_TICKS} (found {})", c.window_ticks),
        );
    }
    let least = usize::from(!later);
    if c.effects.len() < least || c.effects.len() > MAX_COUNTER_EFFECTS {
        issue(
            format!("{at}.effects"),
            format!(
                "{least} to {MAX_COUNTER_EFFECTS} effects (found {})",
                c.effects.len()
            ),
        );
    }
    let mut seen = [false; MAX_COUNTER_EFFECTS];
    for (e, effect) in c.effects.iter().enumerate().take(MAX_COUNTER_EFFECTS) {
        let at = format!("{at}.effects[{e}]");
        let (key, v, lo, hi) = match effect {
            CounterEffect::PullRound { radius_px } | CounterEffect::ClearBullets { radius_px } => (
                "radius_px",
                i64::from(*radius_px),
                1,
                i64::from(MAX_BURST_RADIUS_PX),
            ),
            CounterEffect::RefundEvade {}
            | CounterEffect::FillSizeStacks {}
            | CounterEffect::FireRunes {} => {
                if std::mem::replace(&mut seen[usize::from(effect.name())], true) {
                    issue(at, "the kind already names this effect".into());
                }
                continue;
            }
            CounterEffect::GaugeFillPct { pct } => ("pct", i64::from(*pct), 1, 100),
            CounterEffect::DamagePct { pct } => ("pct", i64::from(*pct), 1, i64::from(MAX_PCT)),
            CounterEffect::StaggerPct { pct } => ("pct", i64::from(*pct), 101, i64::from(MAX_PCT)),
            CounterEffect::StunTicks { ticks } => (
                "ticks",
                i64::from(*ticks),
                1,
                i64::from(validate::limits::MAX_STUN_TICKS),
            ),
            CounterEffect::Advance { distance } => (
                "distance",
                i64::from(distance.0),
                i64::from(Fx::px(1).0),
                i64::from(Fx::px(crate::limits::MAX_ADVANCE_DISTANCE).0),
            ),
        };
        if !(lo..=hi).contains(&v) {
            issue(format!("{at}.{key}"), format!("{lo} to {hi} (found {v})"));
        }
        let slot = &mut seen[usize::from(effect.name())];
        if *slot {
            issue(at, "the kind already names this effect".into());
        }
        *slot = true;
    }
}

pub(super) fn check_step_len(issue: &mut impl FnMut(String, String), at: &str, step: &StepRef) {
    check_step_len_at(issue, &format!("{at}.step"), step);
}

/// A step's length, reported at `path` itself (a list entry such as `.steps[2]`).
pub(super) fn check_step_len_at(
    issue: &mut impl FnMut(String, String),
    path: &str,
    step: &StepRef,
) {
    let StepRef::Path(step) = step else {
        return;
    };
    if step.is_empty() || step.len() > MAX_COMBO_DEPTH as usize {
        issue(
            path.to_string(),
            format!("1 to {MAX_COMBO_DEPTH} presses (found {})", step.len()),
        );
    }
}

/// A value in `lo..=hi`, or an issue at `at.key`.
fn check_range(
    issue: &mut impl FnMut(String, String),
    at: &str,
    key: &str,
    value: u32,
    (lo, hi): (u32, u32),
) {
    if !(lo..=hi).contains(&value) {
        issue(
            format!("{at}.{key}"),
            format!("{lo} to {hi} (found {value})"),
        );
    }
}

/// An encore's numbers, each in its own range, and its step as a step of one to
/// `MAX_COMBO_DEPTH` presses.
pub(super) fn check_encore(issue: &mut impl FnMut(String, String), at: &str, e: &Encore) {
    check_step_len(issue, at, &e.step);
    let skip_most = MAX_COMBO_DEPTH - 2;
    check_range(issue, at, "skip_presses", e.skip_presses, (1, skip_most));
    check_range(
        issue,
        at,
        "ticks",
        e.ticks,
        (1, u32::from(MAX_ENCORE_TICKS)),
    );
    check_range(
        issue,
        at,
        "speed_pct",
        e.speed_pct,
        (1, MAX_ATTACK_SPEED_PCT),
    );
    if let Some(w) = e.wave_presses {
        check_range(issue, at, "wave_presses", w, (1, MAX_COUNTED_PRESSES));
    }
}

/// The knockdown steps' numbers; `later` is whether an earlier level of the path names the kind
/// (it may then list no step, if it names an effect). No member is listed twice in the level.
pub(super) fn check_knockdown(
    issue: &mut impl FnMut(String, String),
    at: &str,
    k: &KnockdownSteps,
    later: bool,
) {
    let least = usize::from(!later);
    if k.steps.len() < least || k.steps.len() > MAX_KNOCKDOWN_STEPS {
        issue(
            format!("{at}.steps"),
            format!(
                "{least} to {MAX_KNOCKDOWN_STEPS} steps (found {})",
                k.steps.len()
            ),
        );
    }
    let mut listed: Vec<Vec<Press>> = Vec::new();
    for (i, step) in k.steps.iter().enumerate().take(MAX_KNOCKDOWN_STEPS) {
        let entry = format!("{at}.steps[{i}]");
        check_step_len_at(issue, &entry, step);
        if claims(&mut listed, step) {
            issue(entry, "the step is already listed".into());
        }
    }
    if let Some(v) = k.vulnerable_after {
        check_range(
            issue,
            at,
            "vulnerable_after",
            v,
            (1, MAX_VULNERABLE_AFTER_TICKS),
        );
    }
    if let Some(p) = k.second_hit_pct {
        check_range(issue, at, "second_hit_pct", p, (1, MAX_PCT));
    }
    if later && k.steps.is_empty() && k.vulnerable_after.is_none() && k.second_hit_pct.is_none() {
        issue(at.to_string(), "the kind names nothing".into());
    }
}

/// The area bonus's numbers; at least one is named.
pub(super) fn check_area_bonus(issue: &mut impl FnMut(String, String), at: &str, a: &AreaBonus) {
    if a.ticks_pct.is_none() && a.range_pct.is_none() && a.proc_pct.is_none() {
        issue(at.to_string(), "the kind names no effect".into());
    }
    if let Some(t) = a.ticks_pct {
        check_range(issue, at, "ticks_pct", t, (1, MAX_PCT));
    }
    if let Some(r) = a.range_pct {
        check_range(issue, at, "range_pct", r, (1, MAX_RANGE_PCT));
    }
    if let Some(p) = a.proc_pct {
        check_range(issue, at, "proc_pct", p, (1, MAX_PROC_PCT));
    }
}

/// A down bonus's `pct`.
pub(super) fn check_down_bonus(issue: &mut impl FnMut(String, String), at: &str, pct: u32) {
    check_range(issue, at, "pct", pct, (1, MAX_PCT));
}

pub(super) fn check_proc_pct(
    issue: &mut impl FnMut(String, String),
    at: &str,
    proc_pct: Option<u32>,
) {
    if let Some(p) = proc_pct.filter(|&p| p > MAX_PROC_PCT) {
        issue(
            format!("{at}.proc_pct"),
            format!("0 to {MAX_PROC_PCT} (found {p})"),
        );
    }
}

/// Whether any member of `step` is already in `taken`; the members are added either way.
fn claims(taken: &mut Vec<Vec<Press>>, step: &StepRef) -> bool {
    let members = step.members();
    let again = members.iter().any(|m| taken.contains(m));
    for member in members {
        if !taken.contains(&member) {
            taken.push(member);
        }
    }
    again
}

/// One grant on a step per style, and one smash effect and one gauge: a second grant on a step an
/// earlier level of the same style granted, or a second smash effect or gauge, is refused at the
/// later one. The spirit, counter bonus, combo tempo and giant trigger are named once per level
/// and may be named at every level of the path.
pub(super) fn check_step_once(issue: &mut impl FnMut(String, String), base: &str, style: &Style) {
    // The steps each list has granted so far, member by member: a group claims all of its own.
    let mut seen: Vec<Vec<Press>> = Vec::new();
    let mut factored: Vec<Vec<Press>> = Vec::new();
    let mut hastened: Vec<Vec<Press>> = Vec::new();
    let mut smash_set = false;
    let mut orbs_set = false;
    let mut bleed_set = false;
    let mut range_set = false;
    let mut proc_set = false;
    let mut stack_set = false;
    let mut gauge_set = false;
    let mut down_bonus_set = false;
    // The members earlier levels list as knockdown steps.
    let mut knocked: Vec<Vec<Press>> = Vec::new();
    // The tempo's folded `max_stacks` and `speed_pct` over the levels so far, once one named it.
    let mut tempo: Option<(u32, u32)> = None;
    let mut levels: Vec<(String, &[StyleKind])> = vec![(format!("{base}.kinds"), &style.kinds)];
    for (field, option) in style_levels(style) {
        if let Some(o) = option {
            levels.push((format!("{base}.{field}[0].kinds"), &o.kinds));
        }
    }
    for (path, kinds) in levels {
        // A path may name the breaker, spirit, counter bonus, combo tempo and giant trigger at
        // every level (their numbers stack in path order), a level only once.
        let mut breaker_here = false;
        let mut spirit_here = false;
        let mut counter_here = false;
        let mut tempo_here = false;
        let mut trigger_here = false;
        let mut encore_here = false;
        let mut knockdown_here = false;
        let mut area_here = false;
        let knocked_before = knocked.clone();
        for (k, kind) in kinds.iter().enumerate().take(MAX_LEVEL_KINDS) {
            match kind {
                StyleKind::Breaker(_) => {
                    if breaker_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a breaker".into(),
                        );
                    }
                    breaker_here = true;
                }
                StyleKind::ReplaceStep { step, .. } => {
                    if claims(&mut seen, step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step is already granted by an earlier level of the style".into(),
                        );
                    }
                }
                StyleKind::SmashEffect { .. } => {
                    if smash_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already sets the smash effect".into(),
                        );
                    }
                    smash_set = true;
                }
                StyleKind::Repeat { .. } => {}
                StyleKind::RepeatBreaksOrbs { .. } => {
                    if orbs_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already breaks orbs".into(),
                        );
                    }
                    orbs_set = true;
                }
                StyleKind::RepeatBleed { .. } => {
                    if bleed_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already bleeds on its repeats".into(),
                        );
                    }
                    bleed_set = true;
                }
                StyleKind::RepeatRangePct { .. } => {
                    if range_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already widens its repeats".into(),
                        );
                    }
                    range_set = true;
                }
                StyleKind::RepeatProcPct { .. } => {
                    if proc_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already sets its repeats' proc chance".into(),
                        );
                    }
                    proc_set = true;
                }
                StyleKind::RepeatSizeStack {} => {
                    if stack_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already stacks size on its repeats".into(),
                        );
                    }
                    stack_set = true;
                }
                StyleKind::ProcFactor { step, .. } => {
                    if claims(&mut factored, step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step already has a proc factor from an earlier level of the style"
                                .into(),
                        );
                    }
                }
                StyleKind::ChargeSooner { step, .. } => {
                    if claims(&mut hastened, step) {
                        issue(
                            format!("{path}[{k}].step"),
                            "the step's charge is already made sooner by an earlier level of the \
                             style"
                                .into(),
                        );
                    }
                }
                StyleKind::Gauge(_) => {
                    if gauge_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already names a gauge".into(),
                        );
                    }
                    gauge_set = true;
                }
                StyleKind::Spirit(_) => {
                    if spirit_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a spirit".into(),
                        );
                    }
                    spirit_here = true;
                }
                StyleKind::CounterBonus(_) => {
                    if counter_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a counter bonus".into(),
                        );
                    }
                    counter_here = true;
                }
                StyleKind::ComboTempoPct(t) => {
                    if tempo_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a combo tempo".into(),
                        );
                    }
                    tempo_here = true;
                    check_tempo_fold(issue, &format!("{path}[{k}]"), t, &mut tempo);
                }
                StyleKind::GiantTrigger(_) => {
                    if trigger_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names a giant trigger".into(),
                        );
                    }
                    trigger_here = true;
                }
                StyleKind::Encore(_) => {
                    if encore_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names an encore".into(),
                        );
                    }
                    encore_here = true;
                }
                StyleKind::KnockdownSteps(n) => {
                    if knockdown_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names knockdown steps".into(),
                        );
                    }
                    knockdown_here = true;
                    for (i, step) in n.steps.iter().enumerate().take(MAX_KNOCKDOWN_STEPS) {
                        let members = step.members();
                        if members.iter().any(|m| knocked_before.contains(m)) {
                            issue(
                                format!("{path}[{k}].steps[{i}]"),
                                "the step is already listed by an earlier level of the style"
                                    .into(),
                            );
                        }
                        for member in members {
                            if !knocked.contains(&member) {
                                knocked.push(member);
                            }
                        }
                    }
                }
                StyleKind::DownBonusPct { .. } => {
                    if down_bonus_set {
                        issue(
                            format!("{path}[{k}]"),
                            "the style already sets the down bonus".into(),
                        );
                    }
                    down_bonus_set = true;
                }
                StyleKind::AreaBonus(_) => {
                    if area_here {
                        issue(
                            format!("{path}[{k}]"),
                            "the level already names an area bonus".into(),
                        );
                    }
                    area_here = true;
                }
            }
        }
    }
}

/// A later level's combo tempo laid over the folded one: `max_stacks` never falls (the stacks
/// held never pass the cap) and `max_stacks` times `speed_pct` stays within the attack speed
/// ceiling at every place of the path, refused at the level that breaks it. The first naming is
/// `check_combo_tempo`'s.
fn check_tempo_fold(
    issue: &mut impl FnMut(String, String),
    at: &str,
    t: &ComboTempo,
    folded: &mut Option<(u32, u32)>,
) {
    let speed = t.effects.iter().find_map(|e| match e {
        TempoEffect::SpeedPct { pct } => Some(*pct),
        _ => None,
    });
    let Some((max, pct)) = *folded else {
        *folded = Some((t.max_stacks, speed.unwrap_or(0)));
        return;
    };
    if t.max_stacks < max {
        issue(
            format!("{at}.max_stacks"),
            format!(
                "at least the earlier level's {max} (found {})",
                t.max_stacks
            ),
        );
    }
    let max = max.max(t.max_stacks);
    let pct = speed
        .filter(|p| (1..=MAX_ATTACK_SPEED_PCT).contains(p))
        .unwrap_or(pct);
    let total = u64::from(max) * u64::from(pct);
    if total > u64::from(MAX_ATTACK_SPEED_PCT) {
        let key = if speed.is_some() {
            "effects"
        } else {
            "max_stacks"
        };
        issue(
            format!("{at}.{key}"),
            format!(
                "max_stacks times speed_pct, folded over the path, is at most {MAX_ATTACK_SPEED_PCT} (found {total})"
            ),
        );
    }
    *folded = Some((max, pct));
}
