//! The expertise gauge, in a stage whose enemies or boss give expertise
//! points: one gauge for the one weapon, from level 0 with no highest level; and its offers
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
//! Each level reached is due an offer, in level order, of the kind the stage's level table gives
//! (`stage::LevelTable`): the weapon's three styles at the table's style level, the
//! chosen style's first and second upgrades at its upgrade levels and its capstone at its
//! capstone level; every other level is a common offer. A common offer draws `offer_size`
//! (`rules.json`) candidates from the weapon's common pool, from the run's generator, leaving out the
//! candidates it already holds; with fewer left it holds those, and with none it does not open.
//! Its options are the candidates, then a reroll of each while the run has rerolls (shared with
//! the stars' offers) and another option could come; never a skip or a banish. Taking an option
//! adds its effect as a reward of that kind does, a range option widening the combo steps as a
//! range reward does, and counts it. No option has a stack limit and none leaves the pool by
//! being taken.
//!
//! A style, upgrade or capstone offer is not drawn: it shows its fixed choices in the content's
//! order, takes nothing from the generator, and its options are the choices alone: no reroll,
//! skip or banish. Taking one applies its effect as a common option's does; the style is held
//! for the run, and its upgrades and capstone are kept in level order. An inline weapon has no
//! expertise content, so none of its levels opens, its style's included; and an upgrade or
//! capstone level with no style chosen opens nothing (fail closed: the table puts the style
//! first, so for a weapon with content a style is always chosen before them).
//!
//! Past the table's last level (at most 10) every level is a common offer; with no pool the
//! levels due are passed over at once, so the offers' cost stays bounded too.
//!
//! In the equipment build no expertise offer opens and nothing stops the world. A common level
//! only raises the weapon level: it adds no stat, takes no option and emits no event. The
//! levels of the table are tree points, held until the player takes them by `TreePick`, in
//! path order (style, upgrade, upgrade, capstone).

use super::World;
use crate::event::DomainEvent;
use crate::expertise::{
    Expertise, ExpertiseEffect, ExpertiseOption, Style, TABLE_LEVELS, WeaponExpertise,
};
use crate::id::ContentId;
use crate::rng::SplitMix64;
use crate::run_stats::StatEffect;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{
    AgentSource, ExpertiseGauge, ExpertiseOffer, PlayerState, Progress, State,
};
use crate::snapshot::{
    ExpertiseCandidateView, ExpertiseOfferKind, ExpertiseOfferView, OfferOption, StatValueView,
    TreeReadyView, TreeStatsView, WeaponStatsView,
};
use crate::stage::{ExpertisePoints, LevelTable};
use crate::weapon::WeaponRef;

use super::entity::Who;
use super::levels;

/// Whether a run of `stage` has the gauge: some wave or the boss gives points.
fn gives_points(stage: &World) -> bool {
    stage.waves.iter().any(|w| w.expertise.is_some())
        || stage.boss.as_ref().is_some_and(|b| b.expertise.is_some())
}

/// The gauge a run of `stage` starts with, at level 0.
pub(super) fn initial(stage: &World, expertise: &Expertise) -> Option<ExpertiseGauge> {
    let taken = || vec![0; pool(stage, expertise).len()];
    gives_points(stage).then(|| ExpertiseGauge {
        level: 0,
        points: 0,
        next: need(expertise, 1, 0),
        offered: 0,
        offer: None,
        taken: taken(),
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

/// The kind of the offer level `level` opens; none at level 0.
fn slot(table: &LevelTable, level: u32) -> Option<ExpertiseOfferKind> {
    use ExpertiseOfferKind::{Capstone, Common, Style, Upgrade};
    Some(if level == 0 {
        return None;
    } else if level == table.style {
        Style
    } else if table.upgrades.contains(&level) {
        Upgrade
    } else if level == table.capstone {
        Capstone
    } else {
        Common
    })
}

/// The expertise content of the one weapon: none for an inline weapon.
fn content<'a>(stage: &World, expertise: &'a Expertise) -> Option<&'a WeaponExpertise> {
    match &stage.player.weapon {
        WeaponRef::Preset { id, .. } => expertise.weapon(id),
        WeaponRef::Inline(_) => None,
    }
}

/// The common pool of the weapon: empty for an inline weapon, which has no expertise content.
fn pool<'a>(stage: &World, expertise: &'a Expertise) -> &'a [ExpertiseOption] {
    content(stage, expertise).map_or(&[], |w| &w.common)
}

/// One choice an offer can hold: a common option, a style, or a choice of a style's path.
pub(super) struct Choice<'a> {
    pub(super) id: &'a ContentId,
    pub(super) effect: ExpertiseEffect,
    pub(super) amount: u32,
}

impl<'a> From<&'a ExpertiseOption> for Choice<'a> {
    fn from(o: &'a ExpertiseOption) -> Self {
        Choice {
            id: &o.id,
            effect: o.effect,
            amount: o.amount,
        }
    }
}

/// The fixed choices of level `level`'s style, upgrade or capstone offer, in the content's
/// order: the styles, or the chosen style's upgrades or capstone. Empty with no content, at a
/// common level, or at an upgrade or capstone level with no style chosen.
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
                pick(style).iter().map(Choice::from).collect()
            })
    };
    match slot(&table, level) {
        Some(ExpertiseOfferKind::Style) => w
            .styles
            .iter()
            .map(|st| Choice {
                id: &st.id,
                effect: st.effect,
                amount: st.amount,
            })
            .collect(),
        Some(ExpertiseOfferKind::Upgrade) if level == table.upgrades[0] => path(|st| &st.upgrade_1),
        Some(ExpertiseOfferKind::Upgrade) => path(|st| &st.upgrade_2),
        Some(ExpertiseOfferKind::Capstone) => path(|st| &st.capstone),
        _ => Vec::new(),
    }
}

/// The choice at place `i` of the offer `o`: in the weapon's common pool, or among the fixed
/// choices.
fn choice_of<'a>(
    stage: &World,
    expertise: &'a Expertise,
    g: &ExpertiseGauge,
    o: &ExpertiseOffer,
    i: u8,
) -> Option<Choice<'a>> {
    let at = usize::from(i);
    match o.kind {
        ExpertiseOfferKind::Common => pool(stage, expertise).get(at).map(Choice::from),
        _ => fixed(stage, expertise, g, o.level).into_iter().nth(at),
    }
}

/// Whether the option at `i` may be offered: in the pool and not in `held`. No option has a
/// stack limit, so one taken before is still offered.
fn eligible(pool: &[ExpertiseOption], i: u8, held: &[u8]) -> bool {
    usize::from(i) < pool.len() && !held.contains(&i)
}

fn places(pool: &[ExpertiseOption]) -> impl Iterator<Item = u8> {
    (0..pool.len()).map_while(|i| u8::try_from(i).ok())
}

/// Draws one option from `pool`, leaving out `held`.
fn draw(pool: &[ExpertiseOption], rng: &mut SplitMix64, held: &[u8]) -> Option<u8> {
    let list: Vec<u8> = places(pool).filter(|i| eligible(pool, *i, held)).collect();
    if list.is_empty() {
        return None;
    }
    let at = rng.below(u32::try_from(list.len()).unwrap_or(u32::MAX));
    list.get(usize::try_from(at).unwrap_or(usize::MAX)).copied()
}

/// Whether some option besides `held` could still be offered from the weapon.
fn any_left(stage: &World, expertise: &Expertise, held: &[u8]) -> bool {
    let pool = pool(stage, expertise);
    places(pool).any(|i| eligible(pool, i, held))
}

// --- the equipment build: automatic common levels ---------------------------------------------

// --- the equipment build: the weapon's stat sets ----------------------------------------------

/// One effect's value at weapon level 1 to 10, in tenths of a percent (`MaxHp`: whole units).
type Row = (StatEffect, [u32; 10]);

/// `floor(max * n / 10)` for `n` in 1..=10: the linear placeholder curve up to the Lv10 value.
const fn lv(max: u32) -> [u32; 10] {
    let mut out = [0; 10];
    let mut n = 1u32;
    while n <= 10 {
        out[(n - 1) as usize] = max * n / 10;
        n += 1;
    }
    out
}

use StatEffect::{AttackPct, AttackSpeedPct, EvadePct, MaxHp, RangePct, SkillCooldownPct};

/// PLACEHOLDER (content row for the pack): a weapon's base set, in force until a tree is picked.
pub(super) const BASE_SETS: [(&str, &[Row]); 2] = [
    (
        "giant_sword",
        &[
            (AttackPct, lv(100)),
            (AttackSpeedPct, lv(100)),
            (MaxHp, lv(100_000)),
        ],
    ),
    (
        "giant_hammer",
        &[
            (AttackPct, lv(100)),
            (AttackSpeedPct, lv(100)),
            (MaxHp, lv(100_000)),
        ],
    ),
];

/// PLACEHOLDER (content row for the pack): the set of each weapon tree (weapon, style), which
/// replaces the base set once the style is picked. The weapon level scales every value.
pub(super) const TREE_SETS: [(&str, &str, &[Row]); 6] = [
    (
        "giant_sword",
        "edge",
        &[
            (AttackPct, lv(500)),
            (AttackSpeedPct, lv(300)),
            (SkillCooldownPct, lv(100)),
            (RangePct, lv(200)),
        ],
    ),
    (
        "giant_sword",
        "tempo",
        &[
            (AttackPct, lv(330)),
            (AttackSpeedPct, lv(1000)),
            (SkillCooldownPct, lv(200)),
            (RangePct, lv(500)),
        ],
    ),
    (
        "giant_sword",
        "reach",
        &[
            (AttackPct, lv(300)),
            (AttackSpeedPct, lv(300)),
            (SkillCooldownPct, lv(100)),
            (RangePct, lv(800)),
        ],
    ),
    (
        "giant_hammer",
        "crush",
        &[
            (AttackPct, lv(600)),
            (AttackSpeedPct, lv(200)),
            (SkillCooldownPct, lv(100)),
            (RangePct, lv(200)),
        ],
    ),
    (
        "giant_hammer",
        "quake",
        &[
            (AttackPct, lv(300)),
            (AttackSpeedPct, lv(200)),
            (SkillCooldownPct, lv(200)),
            (RangePct, lv(800)),
        ],
    ),
    (
        "giant_hammer",
        "bulwark",
        &[
            (AttackPct, lv(200)),
            (SkillCooldownPct, lv(200)),
            (MaxHp, lv(40_000)),
            (EvadePct, lv(200)),
        ],
    ),
];

/// A weapon's stat set at weapon level `level` (0 reads as 1, and past 10 it stops growing):
/// the tree `style` names if it is `Some`, else the weapon's base set. Tenths of a percent per
/// effect in `StatEffect::ALL` order, `MaxHp` whole. A weapon or a style the tables do not name
/// has the empty set. Pure: reads nothing but its arguments.
pub(super) fn tree_stats(
    weapon: &str,
    style: Option<&str>,
    level: u32,
) -> [u32; StatEffect::ALL.len()] {
    let mut out = [0; StatEffect::ALL.len()];
    let rows: &[Row] = match style {
        Some(style) => TREE_SETS
            .iter()
            .find(|(w, s, _)| *w == weapon && *s == style)
            .map_or(&[], |(_, _, rows)| rows),
        None => BASE_SETS
            .iter()
            .find(|(w, _)| *w == weapon)
            .map_or(&[], |(_, rows)| rows),
    };
    let at = usize::try_from(level.clamp(1, 10) - 1).unwrap_or(0);
    for (effect, values) in rows {
        out[*effect as usize] = values[at];
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
    let (WeaponRef::Preset { id, .. }, Some(w)) = (&stage.player.weapon, content(stage, expertise))
    else {
        return [0; StatEffect::ALL.len()];
    };
    match style {
        None => tree_stats(id.as_str(), None, level),
        Some(i) => w
            .styles
            .get(usize::from(i))
            .map_or([0; StatEffect::ALL.len()], |st| {
                tree_stats(id.as_str(), Some(st.id.as_str()), level)
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
    if !stage.equipment_mode {
        return;
    }
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

/// The equipment build's pass over the levels due to `me`: no offer opens, nothing is drawn and
/// no stat is added. The levels of the style path are tree points (held, see the tree rules);
/// every other level only raises the weapon level, which is all the pass records (`offered`).
fn equipment_pass(s: &mut State, me: PlayerIdx) {
    let Some(g) = s.players.career_mut(me).expertise.as_mut() else {
        return;
    };
    g.offered = g.offered.max(g.level);
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
    if !stage.equipment_mode {
        return None;
    }
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
    Some(TreeNext {
        kind,
        level,
        held,
        choices: fixed(stage, expertise, g, level),
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
    let (effect, amount, kind) = (c.effect, c.amount, next.kind);
    // A style adds nothing of its own: the tree's stat set is everything it gives.
    if kind != ExpertiseOfferKind::Style {
        apply(&mut g.range_pct, progress, player, effect, amount);
    }
    if kind == ExpertiseOfferKind::Style {
        let was = (g.style, g.level);
        g.style = Some(place);
        if let Some(p) = progress.as_mut() {
            // The tree's set replaces the base set at the weapon's level.
            write_set(stage, expertise, p, player, Some(was), (g.style, g.level));
        }
    } else {
        g.path.push(place);
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
    if !stage.equipment_mode {
        return None;
    }
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
            })
            .collect(),
    })
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
    // have passed over, so it is held to the longest of the style's three lists.
    let longest = g
        .style
        .and_then(|i| w.styles.get(usize::from(i)))
        .map_or(0, |st| {
            [&st.upgrade_1, &st.upgrade_2, &st.capstone]
                .iter()
                .map(|list| list.len())
                .max()
                .unwrap_or(0)
        });
    if g.path.iter().any(|place| usize::from(*place) >= longest) {
        return Err(out);
    }
    Ok(())
}

/// Opens the offer of the first level due to `me` that has a candidate, unless `me` has an offer
/// open.
pub(super) fn open_due(
    stage: &World,
    expertise: &Expertise,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    if stage.equipment_mode {
        return equipment_pass(s, me);
    }
    if super::offer_open_for(s, me) {
        return;
    }
    let crate::sim::state::Career {
        offer: rng,
        expertise: g,
        ..
    } = s.players.career_mut(me);
    let Some(g) = g.as_mut() else {
        return;
    };
    while g.offer.is_none() && g.offered < g.level {
        if g.offered >= table_levels() && !any_left(stage, expertise, &[]) {
            // Past the style path, nothing can ever be offered again: every level due is
            // passed over.
            g.offered = g.level;
            return;
        }
        g.offered += 1;
        let level = g.offered;
        let Some(kind) = slot(&stage.level_table(), level) else {
            continue;
        };
        let candidates: Vec<u8> = if kind == ExpertiseOfferKind::Common {
            let pool = pool(stage, expertise);
            let mut candidates = Vec::with_capacity(stage.tuning.offer_size);
            while candidates.len() < stage.tuning.offer_size {
                match draw(pool, rng, &candidates) {
                    Some(i) => candidates.push(i),
                    None => break,
                }
            }
            candidates
        } else {
            let n = fixed(stage, expertise, g, level).len();
            (0..n).map_while(|i| u8::try_from(i).ok()).collect()
        };
        if candidates.is_empty() {
            continue;
        }
        let o = ExpertiseOffer {
            level,
            kind,
            candidates,
        };
        events.push(DomainEvent::ExpertiseOfferOpened {
            level,
            kind,
            candidates: o
                .candidates
                .iter()
                .filter_map(|i| choice_of(stage, expertise, g, &o, *i))
                .map(|c| c.id.clone())
                .collect(),
        });
        g.offer = Some(o);
    }
}

/// Rerolls left in the run, shared with the stars' offers; none in a stage without stars.
fn rerolls_left(s: &State, me: PlayerIdx) -> u32 {
    s.players
        .career(me)
        .offers
        .as_ref()
        .map_or(0, |o| o.rerolls_left)
}

/// The options of the open expertise offer, in index order; empty without one.
pub(crate) fn options(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
) -> Vec<OfferOption> {
    let Some((_, o)) = s
        .players
        .career(me)
        .expertise
        .as_ref()
        .and_then(|g| Some((g, g.offer.as_ref()?)))
    else {
        return Vec::new();
    };
    let n = u8::try_from(o.candidates.len()).unwrap_or(u8::MAX);
    let mut out: Vec<OfferOption> = (0..n)
        .map(|candidate| OfferOption::Take { candidate })
        .collect();
    if o.kind == ExpertiseOfferKind::Common
        && rerolls_left(s, me) > 0
        && any_left(stage, expertise, &o.candidates)
    {
        out.extend((0..n).map(|candidate| OfferOption::Reroll { candidate }));
    }
    out
}

/// Carries out `option`, one of `options`, on the open expertise offer.
pub(super) fn choose(
    stage: &World,
    expertise: &Expertise,
    s: &mut State,
    me: PlayerIdx,
    option: OfferOption,
    events: &mut Vec<DomainEvent>,
) {
    let (player, career) = s.players.both_mut(me);
    let crate::sim::state::Career {
        progress,
        offer: rng,
        offers,
        expertise: g,
        ..
    } = career;
    let Some(g) = g.as_mut() else {
        return;
    };
    let Some(mut o) = g.offer.take() else {
        return;
    };
    let pool = pool(stage, expertise);
    match option {
        OfferOption::Take { candidate } if o.kind != ExpertiseOfferKind::Common => {
            let Some(c) = o
                .candidates
                .get(usize::from(candidate))
                .and_then(|i| choice_of(stage, expertise, g, &o, *i))
            else {
                return;
            };
            let (place, id) = (o.candidates[usize::from(candidate)], c.id.clone());
            apply(&mut g.range_pct, progress, player, c.effect, c.amount);
            if o.kind == ExpertiseOfferKind::Style {
                g.style = Some(place);
            } else {
                g.path.push(place);
            }
            events.push(DomainEvent::ExpertiseOptionTaken {
                option: id,
                taken: 1,
            });
        }
        OfferOption::Take { candidate } => {
            let Some(&i) = o.candidates.get(usize::from(candidate)) else {
                return;
            };
            let at = usize::from(i);
            let (Some(n), Some(picked)) = (g.taken.get_mut(at), pool.get(at)) else {
                return;
            };
            *n = n.saturating_add(1);
            let taken = *n;
            apply(
                &mut g.range_pct,
                progress,
                player,
                picked.effect,
                picked.amount,
            );
            events.push(DomainEvent::ExpertiseOptionTaken {
                option: picked.id.clone(),
                taken,
            });
        }
        OfferOption::Reroll { candidate } if o.kind == ExpertiseOfferKind::Common => {
            let slot = usize::from(candidate);
            let left = offers.as_ref().is_some_and(|f| f.rerolls_left > 0);
            let new = if left {
                draw(pool, rng, &o.candidates)
            } else {
                None
            };
            if let (Some(new), Some(f), true) = (new, offers.as_mut(), slot < o.candidates.len()) {
                f.rerolls_left = f.rerolls_left.saturating_sub(1);
                o.candidates[slot] = new;
                events.push(DomainEvent::ExpertiseCandidateReplaced {
                    slot: candidate,
                    option: pool[usize::from(new)].id.clone(),
                });
            }
            g.offer = Some(o);
        }
        // Never among an expertise offer's options; a reroll only among a common one's.
        OfferOption::Reroll { .. } | OfferOption::Skip | OfferOption::Banish { .. } => {
            g.offer = Some(o)
        }
    }
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

/// What the frontend shows of the open expertise offer.
pub(crate) fn offer_view(
    stage: &World,
    expertise: &Expertise,
    s: &State,
    me: PlayerIdx,
) -> Option<ExpertiseOfferView> {
    let g = s.players.career(me).expertise.as_ref()?;
    let o = g.offer.as_ref()?;
    Some(ExpertiseOfferView {
        level: o.level,
        kind: o.kind,
        candidates: o
            .candidates
            .iter()
            .filter_map(|i| {
                let c = choice_of(stage, expertise, g, o, *i)?;
                Some(ExpertiseCandidateView {
                    option: c.id.clone(),
                })
            })
            .collect(),
        options: options(stage, expertise, s, me),
        rerolls_left: rerolls_left(s, me),
    })
}

/// `TABLE_LEVELS` as a level number.
fn table_levels() -> u32 {
    u32::try_from(TABLE_LEVELS).expect("the table is a few levels long")
}
