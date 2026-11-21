//! The style application: the levels of a style's path, a grant on a step, and the weapon a style builds.

use super::{ExpertiseOption, StepGrant, StepRef, Style, StyleKind};
use crate::attack::AttackRef;
use crate::event::Press;
use crate::validate::limits::MAX_DAMAGE;
use crate::weapon::{Branches, RepeatBleed, Step, StepRepeat, Weapon};

/// The levels of a style's path in order, as the simulation takes them: the style pick's own
/// kinds, then the one choice of each upgrade and the capstone.
pub(crate) fn style_levels(style: &Style) -> [(&'static str, Option<&ExpertiseOption>); 3] {
    [
        ("upgrade_1", style.upgrade_1.first()),
        ("upgrade_2", style.upgrade_2.first()),
        ("capstone", style.capstone.first()),
    ]
}

/// The step the `presses` reach from the root of a combo tree.
pub(crate) fn step_mut<'a>(branches: &'a mut Branches, presses: &[Press]) -> Option<&'a mut Step> {
    let (first, rest) = presses.split_first()?;
    let step = match first {
        Press::Main => branches.main.as_deref_mut(),
        Press::Smash => branches.smash.as_deref_mut(),
    }?;
    if rest.is_empty() {
        Some(step)
    } else {
        step_mut(&mut step.next, rest)
    }
}

/// The step the `presses` reach from the root of a combo tree, read only.
pub(crate) fn step_ref<'a>(branches: &'a Branches, presses: &[Press]) -> Option<&'a Step> {
    let (first, rest) = presses.split_first()?;
    let step = match first {
        Press::Main => branches.main.as_deref(),
        Press::Smash => branches.smash.as_deref(),
    }?;
    if rest.is_empty() {
        Some(step)
    } else {
        step_ref(&step.next, rest)
    }
}

/// Applies one `replace_step` grant to the step of `weapon` that `presses` reach from the root
/// of the combo tree: the grant sets the step's charge (clearing its hold) or its hold
/// (clearing its charge). `false`, nothing changed, when no step is there.
pub(crate) fn apply_grant(weapon: &mut Weapon, presses: &[Press], grant: &StepGrant) -> bool {
    let Some(step) = step_mut(&mut weapon.combo, presses) else {
        return false;
    };
    match grant {
        StepGrant::Charge(c) => {
            step.charge = Some(c.clone());
            step.hold = None;
        }
        StepGrant::Hold(h) => {
            step.hold = Some(h.clone());
            step.charge = None;
        }
    }
    true
}

/// Applies a `replace_step` grant to each member of `step` the weapon's tree has. `false`,
/// nothing changed, when it has none.
pub(crate) fn grant_step(weapon: &mut Weapon, step: &StepRef, grant: &StepGrant) -> bool {
    let members = step.members_in(&weapon.combo);
    for member in &members {
        apply_grant(weapon, member, grant);
    }
    !members.is_empty()
}

/// A level's `at_ticks` made `pct` percent sooner: `at * (100 - pct) / 100`, floored, at least 1.
/// A `pct` past 100 counts as 100 (the validator holds it to `MAX_CHARGE_SOONER_PCT`).
pub(crate) fn sooner_at(at: u32, pct: u32) -> u32 {
    let kept = u64::from(100 - pct.min(100));
    u32::try_from(u64::from(at) * kept / 100)
        .unwrap_or(u32::MAX)
        .max(1)
}

/// Makes each level of the charge on the step `presses` reach `pct` percent sooner. `false`,
/// nothing changed, when no step is there or it has no charge.
pub(crate) fn charge_sooner(weapon: &mut Weapon, presses: &[Press], pct: u32) -> bool {
    let Some(charge) = step_mut(&mut weapon.combo, presses).and_then(|s| s.charge.as_mut()) else {
        return false;
    };
    for level in &mut charge.levels {
        level.at_ticks = sooner_at(level.at_ticks, pct);
    }
    true
}

/// Makes the charge of each member of `step` the tree has `pct` percent sooner: the members the
/// tree has and that charge. Returns the members the tree has that have no charge there (none
/// when every one shortened).
pub(crate) fn sooner_step(weapon: &mut Weapon, step: &StepRef, pct: u32) -> Vec<Vec<Press>> {
    let mut uncharged = Vec::new();
    for member in step.members_in(&weapon.combo) {
        if !charge_sooner(weapon, &member, pct) {
            uncharged.push(member);
        }
    }
    uncharged
}

/// `base` with the grants a style gives when it is chosen and `taken` places of its path
/// (0 to 3: the first upgrade, the second, the capstone) are taken, in path order: the style's
/// own kinds, then each taken place's one choice. A grant that finds no step, or a
/// `charge_sooner` that finds no charge, is skipped (the pack check refuses it before a run). The
/// charge's levels come sooner here, at world build: no state, no hash field.
pub(crate) fn weapon_with_style(base: &Weapon, style: &Style, taken: usize) -> Weapon {
    let mut weapon = base.clone();
    let mut bleed = None;
    let places = style_levels(style)
        .into_iter()
        .take(taken)
        .filter_map(|(_, option)| option.map(|o| o.kinds.as_slice()));
    for kinds in std::iter::once(style.kinds.as_slice()).chain(places) {
        for kind in kinds {
            match kind {
                StyleKind::ReplaceStep { step, grant } => {
                    grant_step(&mut weapon, step, grant);
                }
                StyleKind::SmashEffect { effect } => weapon.smash = Some(*effect),
                StyleKind::Repeat {
                    step,
                    after_ticks,
                    repeat_pct,
                    anchor,
                    proc_pct,
                } => {
                    for member in step.members_in(&weapon.combo) {
                        if let Some(s) = step_mut(&mut weapon.combo, &member) {
                            s.repeats.push(StepRepeat {
                                after_ticks: *after_ticks,
                                repeat_pct: u16::try_from(*repeat_pct).unwrap_or(u16::MAX),
                                anchor: *anchor,
                                proc_pct: proc_pct.map(|p| u16::try_from(p).unwrap_or(u16::MAX)),
                                bleed: None,
                            });
                        }
                    }
                }
                // The riders change no step and no rule reads them yet: typed and viewed only.
                StyleKind::RepeatRangePct { .. }
                | StyleKind::RepeatProcPct { .. }
                | StyleKind::RepeatSizeStack {}
                | StyleKind::Encore(_)
                | StyleKind::KnockdownSteps(_)
                | StyleKind::DownBonusPct { .. }
                | StyleKind::AreaBonus(_) => {}
                // Nor does the orb-breaking kind change a step: `World::repeat_break_of`.
                StyleKind::RepeatBreaksOrbs { .. } => {}
                // The repeat bleed is stamped on every repeat once all the kinds are read.
                StyleKind::RepeatBleed {
                    stacks,
                    duration_ticks,
                    damage,
                    max_stacks,
                } => {
                    bleed = Some(RepeatBleed {
                        stacks: u8::try_from(*stacks).unwrap_or(u8::MAX),
                        duration_ticks: u16::try_from(*duration_ticks).unwrap_or(u16::MAX),
                        damage: (*damage).min(MAX_DAMAGE),
                        max_stacks: u8::try_from(*max_stacks).unwrap_or(u8::MAX),
                    });
                }
                StyleKind::ProcFactor { step, proc_pct } => {
                    for member in step.members_in(&weapon.combo) {
                        if let Some(s) = step_mut(&mut weapon.combo, &member) {
                            s.proc_pct = Some(u16::try_from(*proc_pct).unwrap_or(u16::MAX));
                        }
                    }
                }
                // A path with no charge at this place is refused by the pack check.
                StyleKind::ChargeSooner { step, pct } => {
                    sooner_step(&mut weapon, step, *pct);
                }
                // The gauge changes no step: the rules read it beside the weapon (`World::gauge_of`).
                StyleKind::Gauge(_) => {}
                // Nor does the spirit: the rules read it beside the weapon (`World::spirit_of`).
                StyleKind::Spirit(_) => {}
                // Nor does the counter bonus (`World::counter_of`).
                StyleKind::CounterBonus(_) => {}
                // Nor does the combo tempo (`World::tempo_of`).
                StyleKind::ComboTempoPct(_) => {}
                StyleKind::GiantTrigger(_) => {}
                // Nor does the breaker (`World::breaker_of`).
                StyleKind::Breaker(_) => {}
            }
        }
    }
    // A repeat granted by a later level carries the bleed too: it is read when the repeat is
    // queued, so a path that has taken the kind bleeds on every repeat it holds.
    if bleed.is_some() {
        for_each_step_mut(&mut weapon.combo, &mut |step| {
            for repeat in &mut step.repeats {
                repeat.bleed = bleed;
            }
        });
    }
    weapon
}

/// Runs `f` on every step of a combo tree, a step before the steps it reaches.
fn for_each_step_mut(branches: &mut Branches, f: &mut impl FnMut(&mut Step)) {
    for step in [branches.main.as_deref_mut(), branches.smash.as_deref_mut()]
        .into_iter()
        .flatten()
    {
        f(step);
        for_each_step_mut(&mut step.next, f);
    }
}

/// Every attack a weapon's combo and dash trees can start: a step's, a charge level's (and its
/// perfect one), a hold's loop and release.
pub(crate) fn weapon_attacks(weapon: &Weapon) -> Vec<&AttackRef> {
    fn branches<'a>(b: &'a Branches, out: &mut Vec<&'a AttackRef>) {
        for step in [&b.main, &b.smash].into_iter().flatten() {
            out.push(&step.attack);
            if let Some(charge) = &step.charge {
                for level in &charge.levels {
                    out.push(&level.attack);
                    out.extend(level.perfect_attack.as_ref());
                }
            }
            if let Some(hold) = &step.hold {
                out.push(&hold.loop_attack);
                out.push(&hold.release);
            }
            branches(&step.next, out);
        }
    }
    let mut out = Vec::new();
    branches(&weapon.combo, &mut out);
    branches(&weapon.dash, &mut out);
    out
}
