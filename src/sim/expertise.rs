//! The expertise gauge, in a stage whose enemies or boss give expertise
//! points: one gauge for the one weapon, from level 0 with no highest level; and its tree
//! (ADR-REPL-5).
//!
//! A hit by an attack of an A or a B step of the combo adds its target's points per hit, and a
//! kill its points per kill too: an enemy's from its wave, the boss's and its parts' from the
//! boss (a part's break is not a kill). A summon gives none. Other damage (an orbit, the stage
//! skill, a companion) gives none. The setup's gain multiplier is not applied yet: the setup
//! holds no upgrades.
//!
//! Levels are reached once a tick, after the collisions: while the points reach the next
//! level's need, they pay it and the level rises. Levels 1 to 10 need the content's thresholds;
//! each after needs the one before's points times the growth factor, rounded down and held to
//! the content maximum. Once the need stops growing, the levels left are one division, so a
//! tick's cost stays bounded however many levels its points cross.
//!
//! No level opens an offer and nothing stops the world, in every mode. A level only raises the
//! weapon level: it adds no stat, takes no option, draws nothing and emits no event. The levels
//! of the stage's level table (`stage::LevelTable`) are tree points: the weapon's three styles
//! at the style level, the chosen style's first and second upgrades at its upgrade levels and
//! its capstone at its capstone level. A point is held until the player takes it by `TreePick`,
//! in path order (style, upgrade, upgrade, capstone); taking one applies its effect and records
//! its place. An inline weapon has no expertise content, so none of its levels is a tree point;
//! an upgrade or capstone level with no style chosen holds nothing (fail closed: the table
//! puts the style first).
//!
//! Under a stage's `try_styles` every table level is a style pick again; the gauge's `path` is
//! then the log of the picks after the first, and no upgrade or capstone is ever taken.

use super::World;
use crate::expertise::{
    BreakerEffect, BurstEffect, Expertise, ExpertiseEffect, ExpertiseOption, GaugeSpend, SetStat,
    StepGrant, Style, StyleKind, TABLE_LEVELS, TempoEffect, TriggerEffect, WeaponExpertise,
};
use crate::id::ContentId;
use crate::run_stats::StatEffect;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{AgentSource, ExpertiseGauge, PlayerState, Progress, State};
use crate::snapshot::{
    ExpertiseOfferKind, GaugeSpendKind, LevelKindsView, SmashEffectKind, StaggerBurstView,
    StatValueView, StepGrantKind, StyleKindView, TreeReadyView, TreeStatsView, WeaponStatsView,
};
use crate::stage::{ExpertisePoints, LevelTable};
use crate::weapon::{SmashEffect, WeaponRef};

use super::entity::Who;
use super::levels;

/// Whether a run of `stage` has the gauge: some wave or the boss gives points.
fn gives_points(stage: &World) -> bool {
    stage.waves.iter().any(|w| w.expertise.is_some())
        || stage.boss.as_ref().is_some_and(|b| b.expertise.is_some())
}

/// The gauge a run of `stage` starts with, at level 0.
pub(super) fn initial(stage: &World, expertise: &Expertise) -> Option<ExpertiseGauge> {
    gives_points(stage).then(|| ExpertiseGauge {
        level: 0,
        points: 0,
        next: need(expertise, 1, 0),
        range_pct: 0,
        style: None,
        path: Vec::new(),
    })
}

/// The points level `level` needs from the one before, whose need was `before`.
fn need(expertise: &Expertise, level: u32, before: u32) -> u32 {
    let table = expertise.thresholds();
    match usize::try_from(level) {
        Ok(l @ 1..=TABLE_LEVELS) => table.get(l - 1).copied().unwrap_or(expertise.max_points()),
        _ => {
            let grown = u64::from(before) * u64::from(expertise.growth_pct()) / 100;
            u32::try_from(grown)
                .unwrap_or(u32::MAX)
                .min(expertise.max_points())
        }
    }
}

/// A combo step's hit on `target`, which `killed`, adds its points to the gauge.
pub(super) fn credit(stage: &World, s: &mut State, me: PlayerIdx, target: Who, killed: bool) {
    let points = match target {
        Who::Enemy(i) => s.enemies.get(i).and_then(|e| match e.key.source {
            AgentSource::Wave => usize::try_from(e.key.index)
                .ok()
                .and_then(|w| stage.waves.get(w))
                .and_then(|w| w.expertise),
            _ => None,
        }),
        Who::Boss => s.boss_def(stage).and_then(|b| b.expertise),
        Who::BossPart(_) => s
            .boss_def(stage)
            .and_then(|b| b.expertise)
            .map(|p| ExpertisePoints { kill: 0, ..p }),
        Who::Player(_) | Who::Companion(_) => None,
    };
    let (Some(g), Some(p)) = (s.players.career_mut(me).expertise.as_mut(), points) else {
        return;
    };
    let gained = if killed {
        p.hit.saturating_add(p.kill)
    } else {
        p.hit
    };
    g.points = g.points.saturating_add(gained);
}

/// The levels the gauge's points reach this tick.
pub(super) fn level_up(stage: &World, expertise: &Expertise, s: &mut State, me: PlayerIdx) {
    let (player, career) = s.players.both_mut(me);
    let Some(g) = career.expertise.as_mut() else {
        return;
    };
    let was = (g.style, g.level);
    raise_levels(expertise, g);
    if g.level != was.1 {
        if let Some(p) = career.progress.as_mut() {
            write_set(stage, expertise, p, player, Some(was), (g.style, g.level));
        }
    }
}

/// Raises the gauge by every level its points reach.
fn raise_levels(expertise: &Expertise, g: &mut ExpertiseGauge) {
    while g.next > 0 && g.points >= g.next {
        let reached = g.level.saturating_add(1);
        let after = need(expertise, reached.saturating_add(1), g.next);
        if reached > table_levels() && after == g.next {
            // Every level from here needs as much: the rest is one division.
            g.level = g.level.saturating_add(g.points / g.next);
            g.points %= g.next;
            return;
        }
        g.points -= g.next;
        g.level = reached;
        g.next = after;
    }
}

/// The kind of the tree level `level` is; none at level 0 and at a level that is no tree level.
fn slot(table: &LevelTable, level: u32) -> Option<ExpertiseOfferKind> {
    use ExpertiseOfferKind::{Capstone, Style, Upgrade};
    if level == 0 {
        None
    } else if level == table.style {
        Some(Style)
    } else if table.upgrades.contains(&level) {
        Some(Upgrade)
    } else if level == table.capstone {
        Some(Capstone)
    } else {
        None
    }
}

/// The expertise content of the one weapon: none for an inline weapon.
fn content<'a>(stage: &World, expertise: &'a Expertise) -> Option<&'a WeaponExpertise> {
    match &stage.player.weapon {
        WeaponRef::Preset { id, .. } => expertise.weapon(id),
        WeaponRef::Inline(_) => None,
    }
}

/// One choice of the tree: a style, or a choice of a style's path.
pub(super) struct Choice<'a> {
    /// Where the choice sits: a style's place in the weapon's styles (the offer may skip locked
    /// ones, so it is not the offered index), else the index in its list.
    pub(super) place: u8,
    pub(super) id: &'a ContentId,
    pub(super) effect: ExpertiseEffect,
    pub(super) amount: u32,
    pub(super) kinds: &'a [StyleKind],
}

impl<'a> Choice<'a> {
    fn option(place: usize, o: &'a ExpertiseOption) -> Self {
        Choice {
            place: u8::try_from(place).unwrap_or(u8::MAX),
            id: &o.id,
            effect: o.effect,
            amount: o.amount,
            kinds: &o.kinds,
        }
    }
}

// The offered-styles mask (`World::styles_offered`) is one bit for each style.
const _: () = assert!(crate::expertise::STYLES <= 8);

/// Whether the style at place `at` is offered under the setup's mask (`World::styles_offered`).
fn style_offered(mask: Option<u8>, at: usize) -> bool {
    mask.is_none_or(|m| {
        u32::try_from(at).is_ok_and(|n| m.checked_shr(n).is_some_and(|b| b & 1 == 1))
    })
}

/// The fixed choices of level `level`'s style, upgrade or capstone point, in the content's
/// order: the styles, or the chosen style's upgrades or capstone. Empty with no content, at a
/// level that is no tree level, or at an upgrade or capstone level with no style chosen.
fn fixed<'a>(
    stage: &World,
    expertise: &'a Expertise,
    g: &ExpertiseGauge,
    level: u32,
) -> Vec<Choice<'a>> {
    let Some(w) = content(stage, expertise) else {
        return Vec::new();
    };
    let table = stage.level_table();
    let path = |pick: fn(&'a Style) -> &'a Vec<ExpertiseOption>| {
        g.style
            .and_then(|i| w.styles.get(usize::from(i)))
            .map_or_else(Vec::new, |style| {
                pick(style)
                    .iter()
                    .enumerate()
                    .map(|(at, o)| Choice::option(at, o))
                    .collect()
            })
    };
    // Under `try_styles` every table level is a style pick.
    let kind = slot(&table, level).map(|k| {
        if stage.try_styles {
            ExpertiseOfferKind::Style
        } else {
            k
        }
    });
    match kind {
        Some(ExpertiseOfferKind::Style) => w
            .styles
            .iter()
            .enumerate()
            .filter(|(at, _)| style_offered(stage.styles_offered, *at))
            .map(|(at, st)| Choice {
                place: u8::try_from(at).unwrap_or(u8::MAX),
                id: &st.id,
                effect: st.effect,
                amount: st.amount,
                kinds: &st.kinds,
            })
            .collect(),
        Some(ExpertiseOfferKind::Upgrade) if level == table.upgrades[0] => path(|st| &st.upgrade_1),
        Some(ExpertiseOfferKind::Upgrade) => path(|st| &st.upgrade_2),
        Some(ExpertiseOfferKind::Capstone) => path(|st| &st.capstone),
        _ => Vec::new(),
    }
}

// --- the equipment build: the weapon's stat sets ----------------------------------------------

/// A weapon's stat set at weapon level `level` (0 reads as 1, and past 10 it stops growing),
/// read from the pack: the tree `style` names if it is `Some`, else the weapon's base set.
/// Tenths of a percent per effect in `StatEffect::ALL` order, `MaxHp` whole. No weapon, a style
/// the weapon lacks, or a set the pack leaves out is the empty set. Pure: reads nothing but its
/// arguments.
pub(super) fn tree_stats(
    weapon: Option<&WeaponExpertise>,
    style: Option<&str>,
    level: u32,
) -> [u32; StatEffect::ALL.len()] {
    let mut out = [0; StatEffect::ALL.len()];
    let set: &[SetStat] = match (weapon, style) {
        (Some(w), Some(style)) => w
            .styles
            .iter()
            .find(|st| st.id.as_str() == style)
            .map_or(&[], |st| &st.stats),
        (Some(w), None) => &w.base,
        (None, _) => &[],
    };
    let at = usize::try_from(level.clamp(1, 10) - 1).unwrap_or(0);
    for stat in set {
        // `by_level` has exactly ten entries (validated); a short list reads 0.
        out[stat.effect as usize] = stat.by_level.get(at).copied().unwrap_or(0);
    }
    out
}

/// The set of the run's weapon at `level` with the style `style` (a place in its styles) chosen:
/// empty for an inline weapon, a weapon with no expertise content, or a style it lacks.
pub(super) fn set_for(
    stage: &World,
    expertise: &Expertise,
    style: Option<u8>,
    level: u32,
) -> [u32; StatEffect::ALL.len()] {
    let w = content(stage, expertise);
    match style {
        None => tree_stats(w, None, level),
        Some(i) => w
            .and_then(|w| w.styles.get(usize::from(i)))
            .map_or([0; StatEffect::ALL.len()], |st| {
                tree_stats(w, Some(st.id.as_str()), level)
            }),
    }
}

/// Writes the weapon's set for `now` (style place, weapon level) over the one for `was` (`None`
/// at the start of a run: nothing written yet): the tenths are assigned, the hit points by
/// the difference through `add_effect` / `remove_effect`, so every other source in `totals` is
/// untouched and the set is recomputed, never stacked. Equipment mode with a `Progress` only.
pub(super) fn write_set(
    stage: &World,
    expertise: &Expertise,
    progress: &mut Progress,
    player: &mut PlayerState,
    was: Option<(Option<u8>, u32)>,
    now: (Option<u8>, u32),
) {
    let before = was.map_or([0; StatEffect::ALL.len()], |(style, level)| {
        set_for(stage, expertise, style, level)
    });
    let after = set_for(stage, expertise, now.0, now.1);
    for effect in StatEffect::ALL {
        let at = effect as usize;
        if effect == StatEffect::MaxHp {
            match after[at].cmp(&before[at]) {
                core::cmp::Ordering::Greater => {
                    levels::add_effect(progress, player, effect, after[at] - before[at]);
                }
                core::cmp::Ordering::Less => {
                    levels::remove_effect(progress, player, effect, before[at] - after[at]);
                }
                core::cmp::Ordering::Equal => {}
            }
        } else {
            progress.set_tenths[at] = after[at];
        }
    }
}

/// The start of a run: slot `me`'s base set at weapon level 1, before anything else is written.
pub(super) fn start_set(stage: &World, expertise: &Expertise, s: &mut State, me: PlayerIdx) {
    let (player, career) = s.players.both_mut(me);
    if let (Some(g), Some(p)) = (career.expertise.as_ref(), career.progress.as_mut()) {
        write_set(stage, expertise, p, player, None, (g.style, g.level));
    }
}

// --- the equipment build: tree points ----------------------------------------------------------

/// Why a tree pick is refused (the engine's `ActionError` maps it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeError {
    /// No point is held: none earned, all taken, no gauge, or no expertise content.
    NoTreePoint,
    /// `option` is not below the `options` the next pick has.
    NoTreeOption { option: u32, options: u32 },
}

/// How many tree points the levels up to `level` have earned: the table's levels reached.
pub(super) fn tree_earned(table: &LevelTable, level: u32) -> u8 {
    let n = table.levels().iter().filter(|l| **l <= level).count();
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// How many tree points the gauge has taken: the style, then the path.
fn tree_taken(g: &ExpertiseGauge) -> u8 {
    u8::from(g.style.is_some()).saturating_add(u8::try_from(g.path.len()).unwrap_or(u8::MAX))
}

/// The next tree pick of a gauge that holds a point.
pub(super) struct TreeNext<'a> {
    pub(super) kind: ExpertiseOfferKind,
    /// The table level of this pick.
    pub(super) level: u32,
    /// The points held, at least 1.
    pub(super) held: u8,
    /// The choices, in the order `TreePick.option` indexes.
    pub(super) choices: Vec<Choice<'a>>,
}

/// The next tree pick, or none: not the equipment build, no expertise content, or no point held.
/// The points are derived (held = earned - taken) and taken in path order.
pub(super) fn tree_next<'a>(
    stage: &World,
    expertise: &'a Expertise,
    g: &ExpertiseGauge,
) -> Option<TreeNext<'a>> {
    content(stage, expertise)?;
    let table = stage.level_table();
    let taken = tree_taken(g);
    let held = tree_earned(&table, g.level).saturating_sub(taken);
    if held == 0 {
        return None;
    }
    let (kind, level) = match taken {
        0 => (ExpertiseOfferKind::Style, table.style),
        1 => (ExpertiseOfferKind::Upgrade, table.upgrades[0]),
        2 => (ExpertiseOfferKind::Upgrade, table.upgrades[1]),
        3 => (ExpertiseOfferKind::Capstone, table.capstone),
        _ => return None,
    };
    let kind = if stage.try_styles {
        ExpertiseOfferKind::Style
    } else {
        kind
    };
    let choices = fixed(stage, expertise, g, level);
    // No option, no waiting point: a run with no style unlocked plays the plain base.
    if choices.is_empty() {
        return None;
    }
    Some(TreeNext {
        kind,
        level,
        held,
        choices,
    })
}

/// Whether `me` could take choice `option` of the next tree pick. Read-only.
pub(super) fn check_tree_pick(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    let g = s.players.career(me).expertise.as_ref();
    let next = g
        .and_then(|g| tree_next(stage, expertise, g))
        .ok_or(TreeError::NoTreePoint)?;
    let options = u32::try_from(next.choices.len()).unwrap_or(u32::MAX);
    if option >= options {
        return Err(TreeError::NoTreeOption { option, options });
    }
    Ok(())
}

/// Takes choice `option` of the next tree pick: its effect, and the style or the path. No event,
/// no draw.
pub(super) fn tree_pick(
    stage: &World,
    expertise: &Expertise,
    s: &mut State,
    me: PlayerIdx,
    option: u32,
) -> Result<(), TreeError> {
    check_tree_pick(stage, expertise, s, me, option)?;
    let (player, career) = s.players.both_mut(me);
    let crate::sim::state::Career {
        progress,
        expertise: g,
        ..
    } = career;
    let Some(g) = g.as_mut() else {
        return Err(TreeError::NoTreePoint);
    };
    let Some(next) = tree_next(stage, expertise, g) else {
        return Err(TreeError::NoTreePoint);
    };
    let place = u8::try_from(option).map_err(|_| TreeError::NoTreeOption {
        option,
        options: u32::try_from(next.choices.len()).unwrap_or(u32::MAX),
    })?;
    let Some(c) = next.choices.get(usize::from(place)) else {
        return Err(TreeError::NoTreePoint);
    };
    let (effect, amount, kind, at) = (c.effect, c.amount, next.kind, c.place);
    // Points spent before this pick: under `try_styles` the log in `path` counts them.
    let spent = tree_taken(g);
    // A style adds nothing of its own: the tree's stat set is everything it gives.
    if kind != ExpertiseOfferKind::Style {
        apply(&mut g.range_pct, progress, player, effect, amount);
    }
    if kind == ExpertiseOfferKind::Style {
        let was = (g.style, g.level);
        g.style = Some(at);
        if (stage.has_gauge
            || stage.has_spirit
            || stage.has_counter_bonus
            || stage.has_tempo
            || stage.has_trigger)
            && was.0 != Some(at)
        {
            // The new style's bar starts empty; a style with no gauge reads empty, its spirit
            // level starts at 0 (a style with no spirit reads 0), and an old style's counter
            // window and mark do not carry over.
            player.combat.counter_window = 0;
            player.combat.counter_bonus_step = 0;
            player.combat.weapon_gauge = 0;
            player.combat.gauge_step_fill = 0;
            player.combat.spirit_level = 0;
            player.combat.spirit_decay = 0;
            // And the old style's combo tempo stacks.
            player.combat.combo_tempo = 0;
            // And its giant trigger's ticks, with no event.
            player.combat.giant_trigger = 0;
        }
        // Under `try_styles` the gauge's path is the log of the picks after the first, so the
        // points held stay derived; the new style's upgrade path starts over (`weapon_of`).
        if stage.try_styles && spent > 0 {
            g.path.push(at);
        }
        if let Some(p) = progress.as_mut() {
            // The tree's set replaces the base set at the weapon's level.
            write_set(stage, expertise, p, player, Some(was), (g.style, g.level));
        }
    } else {
        g.path.push(at);
    }
    Ok(())
}

/// What the frontend shows of `me`'s waiting tree point: none in the offers build or while no
/// point is held.
pub(crate) fn tree_ready_view(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
) -> Option<TreeReadyView> {
    let g = s.players.career(me).expertise.as_ref()?;
    let next = tree_next(stage, expertise, g)?;
    Some(TreeReadyView {
        kind: next.kind,
        level: next.level,
        held: next.held,
        options: next.choices.iter().map(|c| c.id.clone()).collect(),
        kinds: if next.choices.iter().any(|c| !c.kinds.is_empty()) {
            next.choices.iter().map(|c| kind_views(c.kinds)).collect()
        } else {
            Vec::new()
        },
    })
}

/// The stat sets of `me`'s weapon for the snapshot: none outside the equipment build, without a
/// gauge or without expertise content. Reads, writes and hashes nothing.
pub(crate) fn weapon_stats_view(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
) -> Option<WeaponStatsView> {
    let g = s.players.career(me).expertise.as_ref()?;
    let w = content(stage, expertise)?;
    let level = g.level.clamp(1, 10);
    let next = (level + 1).min(10);
    let list = |style: Option<u8>| -> Vec<StatValueView> {
        let (now, then) = (
            set_for(stage, expertise, style, level),
            set_for(stage, expertise, style, next),
        );
        StatEffect::ALL
            .into_iter()
            .filter(|f| now[*f as usize] != 0 || then[*f as usize] != 0)
            .map(|effect| StatValueView {
                effect,
                now: now[effect as usize],
                next: then[effect as usize],
            })
            .collect()
    };
    Some(WeaponStatsView {
        level,
        active: g
            .style
            .and_then(|i| w.styles.get(usize::from(i)))
            .map(|st| st.id.clone()),
        base: list(None),
        trees: (0..w.styles.len())
            .filter_map(|i| u8::try_from(i).ok())
            .map(|i| TreeStatsView {
                style: w.styles[usize::from(i)].id.clone(),
                stats: list(Some(i)),
                grants: grant_views(&w.styles[usize::from(i)]),
            })
            .collect(),
    })
}

/// The compact view of a level's kinds.
fn kind_views(kinds: &[StyleKind]) -> Vec<StyleKindView> {
    kinds
        .iter()
        .map(|kind| match kind {
            StyleKind::ReplaceStep { step, grant } => StyleKindView::ReplaceStep {
                step: step.clone(),
                grant: match grant {
                    StepGrant::Charge(_) => StepGrantKind::Charge,
                    StepGrant::Hold(_) => StepGrantKind::Hold,
                },
            },
            StyleKind::SmashEffect { effect } => StyleKindView::SmashEffect {
                effect: match effect {
                    SmashEffect::Pull { .. } => SmashEffectKind::Pull,
                    SmashEffect::Knockback { .. } => SmashEffectKind::Knockback,
                    SmashEffect::Bleed {} => SmashEffectKind::Bleed,
                    SmashEffect::Stun { .. } => SmashEffectKind::Stun,
                    SmashEffect::Reap { .. } => SmashEffectKind::Reap,
                },
            },
            StyleKind::Repeat {
                step,
                after_ticks,
                repeat_pct,
                anchor,
                proc_pct,
            } => StyleKindView::Repeat {
                step: step.clone(),
                after_ticks: *after_ticks,
                repeat_pct: *repeat_pct,
                anchor: *anchor,
                proc_pct: *proc_pct,
            },
            StyleKind::ProcFactor { step, proc_pct } => StyleKindView::ProcFactor {
                step: step.clone(),
                proc_pct: *proc_pct,
            },
            StyleKind::Gauge(g) => StyleKindView::Gauge {
                max: g.max,
                spend: match g.spend {
                    GaugeSpend::FullSmash { .. } => GaugeSpendKind::FullSmash,
                },
            },
            StyleKind::Spirit(s) => StyleKindView::Spirit {
                max_level: u8::try_from(s.levels.len()).unwrap_or(u8::MAX),
                decay_ticks: s.decay_ticks,
            },
            StyleKind::CounterBonus(c) => StyleKindView::CounterBonus {
                window_ticks: c.window_ticks,
            },
            StyleKind::ComboTempoPct(t) => StyleKindView::ComboTempoPct {
                max_stacks: t.max_stacks,
                pct_per_stack: t
                    .effects
                    .iter()
                    .find_map(|e| match e {
                        TempoEffect::SpeedPct { pct } => Some(*pct),
                        _ => None,
                    })
                    .unwrap_or(0),
                armour_at_full: t
                    .effects
                    .iter()
                    .any(|e| matches!(e, TempoEffect::SuperArmourAtFull {})),
            },
            StyleKind::GiantTrigger(t) => {
                let mut view = StyleKindView::GiantTrigger {
                    ticks: t.ticks,
                    range_pct: 0,
                    smash_pct: 0,
                    rally_pct: 0,
                    super_armour: false,
                };
                if let StyleKindView::GiantTrigger {
                    range_pct,
                    smash_pct,
                    rally_pct,
                    super_armour,
                    ..
                } = &mut view
                {
                    for effect in &t.effects {
                        match effect {
                            TriggerEffect::RangePct { pct } => *range_pct = *pct,
                            TriggerEffect::SmashPct { pct } => *smash_pct = *pct,
                            TriggerEffect::RallyPct { pct } => *rally_pct = *pct,
                            TriggerEffect::SuperArmour {} => *super_armour = true,
                        }
                    }
                }
                view
            }
            StyleKind::Breaker(b) => {
                let (mut stagger_pct, mut poise_pct, mut part_break_pct, mut burst) =
                    (0, 0, 0, None);
                for effect in &b.effects {
                    match effect {
                        BreakerEffect::StaggerPct { pct } => stagger_pct = *pct,
                        BreakerEffect::PoisePct { pct } => poise_pct = *pct,
                        BreakerEffect::PartBreakPct { pct } => part_break_pct = *pct,
                        BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                            let mut v = StaggerBurstView {
                                radius_px: *radius_px,
                                damage_pct: 0,
                                stun_ticks: 0,
                                knock_down: false,
                            };
                            for e in effects {
                                match e {
                                    BurstEffect::DamagePct { pct } => v.damage_pct = *pct,
                                    BurstEffect::StunTicks { ticks } => v.stun_ticks = *ticks,
                                    BurstEffect::KnockDown {} => v.knock_down = true,
                                }
                            }
                            burst = Some(v);
                        }
                    }
                }
                StyleKindView::Breaker {
                    stagger_pct,
                    poise_pct,
                    part_break_pct,
                    burst,
                }
            }
        })
        .collect()
}

/// What a style grants, level by level: its own pick, then the one choice of each place of its
/// path; empty when no level names a kind.
fn grant_views(style: &Style) -> Vec<LevelKindsView> {
    let mut levels = vec![LevelKindsView {
        offer: ExpertiseOfferKind::Style,
        option: style.id.clone(),
        kinds: kind_views(&style.kinds),
    }];
    for (offer, option) in [
        (ExpertiseOfferKind::Upgrade, style.upgrade_1.first()),
        (ExpertiseOfferKind::Upgrade, style.upgrade_2.first()),
        (ExpertiseOfferKind::Capstone, style.capstone.first()),
    ] {
        if let Some(o) = option {
            levels.push(LevelKindsView {
                offer,
                option: o.id.clone(),
                kinds: kind_views(&o.kinds),
            });
        }
    }
    if levels.iter().all(|l| l.kinds.is_empty()) {
        Vec::new()
    } else {
        levels
    }
}

/// Whether a loaded gauge could have been reached: the tree taken is within the points the level
/// earned, the style is one of the weapon's and each place of the path is one of its level's
/// choices. A message for the first it fails; the check does not change the layout.
pub(crate) fn check_tree(
    stage: &World,
    expertise: &Expertise,
    g: &ExpertiseGauge,
) -> Result<(), &'static str> {
    let table = stage.level_table();
    let taken = tree_taken(g);
    let past = "expertise tree taken past the level";
    if taken > tree_earned(&table, g.level) || (g.style.is_none() && !g.path.is_empty()) {
        return Err(past);
    }
    let out = "expertise tree place out of range";
    let Some(w) = content(stage, expertise) else {
        return if taken == 0 { Ok(()) } else { Err(past) };
    };
    if g.style.is_some_and(|i| usize::from(i) >= w.styles.len()) {
        return Err(out);
    }
    // A place indexes the list of the level it was taken at, which an offers run's out player may
    // have passed over, so it is held to the longest of the style's three lists; under
    // `try_styles` it is a style's place, held to the weapon's styles.
    let longest = if stage.try_styles {
        w.styles.len()
    } else {
        g.style
            .and_then(|i| w.styles.get(usize::from(i)))
            .map_or(0, |st| {
                [&st.upgrade_1, &st.upgrade_2, &st.capstone]
                    .iter()
                    .map(|list| list.len())
                    .max()
                    .unwrap_or(0)
            })
    };
    if g.path.iter().any(|place| usize::from(*place) >= longest) {
        return Err(out);
    }
    Ok(())
}

/// Adds `amount` of `effect`, as a reward of that kind does; a range option to the combo
/// steps' widening.
fn apply(
    range_pct: &mut u32,
    progress: &mut Option<crate::sim::state::Progress>,
    player: &mut crate::sim::state::PlayerState,
    effect: ExpertiseEffect,
    amount: u32,
) {
    match effect {
        ExpertiseEffect::RangePct => *range_pct = range_pct.saturating_add(amount),
        effect => {
            if let (Some(effect), Some(p)) = (effect.stat(), progress.as_mut()) {
                levels::add_effect(p, player, effect, amount);
            }
        }
    }
}

/// The ID of the style chosen, and of the style's path taken, in level order.
pub(crate) fn styles_view(
    stage: &World,
    expertise: &Expertise,
    g: &ExpertiseGauge,
) -> (Option<ContentId>, Vec<ContentId>) {
    let style = content(stage, expertise).and_then(|w| {
        let chosen = g.style?;
        Some(w.styles.get(usize::from(chosen))?.id.clone())
    });
    if stage.try_styles {
        // The path is the log of the picks, not an upgrade path.
        return (style, Vec::new());
    }
    let table = stage.level_table();
    let levels = [table.upgrades[0], table.upgrades[1], table.capstone];
    let path = levels
        .into_iter()
        .zip(&g.path)
        .filter_map(|(level, i)| {
            fixed(stage, expertise, g, level)
                .into_iter()
                .nth(usize::from(*i))
                .map(|c| c.id.clone())
        })
        .collect();
    (style, path)
}

/// `TABLE_LEVELS` as a level number.
fn table_levels() -> u32 {
    u32::try_from(TABLE_LEVELS).expect("the table is a few levels long")
}
