//! The expertise gauge (ADR-106 §1 to §3), in a stage whose enemies or boss give expertise
//! points: one gauge for the one weapon, from level 0 with no highest level; and its offers
//! (ADR-106 §4 to §6; ADR-079 §2, §3).
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
//! (`stage::LevelTable`, ADR-106 §4): the weapon's three styles at the table's style level, the
//! chosen style's first and second upgrades at its upgrade levels and its capstone at its
//! capstone level; every other level is a common offer. A common offer draws `OFFER_SIZE`
//! candidates from the weapon's common pool, from the run's generator, leaving out the
//! candidates it already holds; with fewer left it holds those, and with none it does not open.
//! Its options are the candidates, then a reroll of each while the run has rerolls (shared with
//! the stars' offers) and another option could come; never a skip or a banish. Taking an option
//! adds its effect as a reward of that kind does, a range option widening the combo steps as a
//! range reward does, and counts it. No option has a stack limit and none leaves the pool by
//! being taken (ADR-099).
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

use crate::engine::{AgentSource, ExpertiseGauge, ExpertiseOffer, State};
use crate::event::DomainEvent;
use crate::expertise::{
    Expertise, ExpertiseEffect, ExpertiseOption, Style, TABLE_LEVELS, WeaponExpertise,
};
use crate::id::ContentId;
use crate::rng::SplitMix64;
use crate::snapshot::{
    ExpertiseCandidateView, ExpertiseOfferKind, ExpertiseOfferView, OfferOption,
};
use crate::stage::{ExpertisePoints, LevelTable, Stage};
use crate::weapon::WeaponRef;

use super::entity::Who;
use super::levels;
use super::stars::OFFER_SIZE;

/// Whether a run of `stage` has the gauge: some wave or the boss gives points.
fn gives_points(stage: &Stage) -> bool {
    stage.waves.iter().any(|w| w.expertise.is_some())
        || stage.boss.as_ref().is_some_and(|b| b.expertise.is_some())
}

/// The gauge a run of `stage` starts with, at level 0.
pub(super) fn initial(stage: &Stage, expertise: &Expertise) -> Option<ExpertiseGauge> {
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
pub(super) fn credit(stage: &Stage, s: &mut State, target: Who, killed: bool) {
    let points = match target {
        Who::Enemy(i) => s.enemies.get(i).and_then(|e| match e.key.source {
            AgentSource::Wave => usize::try_from(e.key.index)
                .ok()
                .and_then(|w| stage.waves.get(w))
                .and_then(|w| w.expertise),
            _ => None,
        }),
        Who::Boss => stage.boss.as_ref().and_then(|b| b.expertise),
        Who::BossPart(_) => stage
            .boss
            .as_ref()
            .and_then(|b| b.expertise)
            .map(|p| ExpertisePoints { kill: 0, ..p }),
        Who::Player | Who::Companion(_) => None,
    };
    let (Some(g), Some(p)) = (s.expertise.as_mut(), points) else {
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
pub(super) fn level_up(expertise: &Expertise, s: &mut State) {
    let Some(g) = s.expertise.as_mut() else {
        return;
    };
    while g.next > 0 && g.points >= g.next {
        let reached = g.level.saturating_add(1);
        let after = need(expertise, reached.saturating_add(1), g.next);
        if reached > TABLE_LEVELS as u32 && after == g.next {
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

/// The kind of the offer level `level` opens (ADR-106 §4); none at level 0.
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
fn content<'a>(stage: &Stage, expertise: &'a Expertise) -> Option<&'a WeaponExpertise> {
    match &stage.player.weapon {
        WeaponRef::Preset { id, .. } => expertise.weapon(id),
        WeaponRef::Inline(_) => None,
    }
}

/// The common pool of the weapon: empty for an inline weapon, which has no expertise content.
fn pool<'a>(stage: &Stage, expertise: &'a Expertise) -> &'a [ExpertiseOption] {
    content(stage, expertise).map_or(&[], |w| &w.common)
}

/// One choice an offer can hold: a common option, a style, or a choice of a style's path.
struct Choice<'a> {
    id: &'a ContentId,
    effect: ExpertiseEffect,
    amount: u32,
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
    stage: &Stage,
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
    stage: &Stage,
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
/// stack limit, so one taken before is still offered (ADR-099).
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
fn any_left(stage: &Stage, expertise: &Expertise, held: &[u8]) -> bool {
    let pool = pool(stage, expertise);
    places(pool).any(|i| eligible(pool, i, held))
}

/// Opens the offer of the first level due that has a candidate, unless an offer is open.
pub(super) fn open_due(
    stage: &Stage,
    expertise: &Expertise,
    s: &mut State,
    events: &mut Vec<DomainEvent>,
) {
    if super::offer_open(s) {
        return;
    }
    let State {
        rng, expertise: g, ..
    } = s;
    let Some(g) = g.as_mut() else {
        return;
    };
    while g.offer.is_none() && g.offered < g.level {
        if g.offered >= TABLE_LEVELS as u32 && !any_left(stage, expertise, &[]) {
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
            let mut candidates = Vec::with_capacity(OFFER_SIZE);
            while candidates.len() < OFFER_SIZE {
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
fn rerolls_left(s: &State) -> u32 {
    s.stars.as_ref().map_or(0, |f| f.rerolls_left)
}

/// The options of the open expertise offer, in index order; empty without one.
pub(crate) fn options(stage: &Stage, expertise: &Expertise, s: &State) -> Vec<OfferOption> {
    let Some((_, o)) = s
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
        && rerolls_left(s) > 0
        && any_left(stage, expertise, &o.candidates)
    {
        out.extend((0..n).map(|candidate| OfferOption::Reroll { candidate }));
    }
    out
}

/// Carries out `option`, one of `options`, on the open expertise offer.
pub(super) fn choose(
    stage: &Stage,
    expertise: &Expertise,
    s: &mut State,
    option: OfferOption,
    events: &mut Vec<DomainEvent>,
) {
    let State {
        rng,
        stars,
        progress,
        player,
        expertise: g,
        ..
    } = s;
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
            let left = stars.as_ref().is_some_and(|f| f.rerolls_left > 0);
            let new = if left {
                draw(pool, rng, &o.candidates)
            } else {
                None
            };
            if let (Some(new), Some(f), true) = (new, stars.as_mut(), slot < o.candidates.len()) {
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
    progress: &mut Option<crate::engine::Progress>,
    player: &mut crate::engine::PlayerState,
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
    stage: &Stage,
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
    stage: &Stage,
    expertise: &Expertise,
    s: &State,
) -> Option<ExpertiseOfferView> {
    let g = s.expertise.as_ref()?;
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
        options: options(stage, expertise, s),
        rerolls_left: rerolls_left(s),
    })
}
