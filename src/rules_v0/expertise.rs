//! The expertise gauge (ADR-078 §1 to §3), in a stage whose enemies or boss give expertise
//! points: one gauge for both weapons, from level 0 with no highest level; and its offers
//! (ADR-078 §4, §6; ADR-079 §2, §3).
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
//! Each level reached is due an offer, in level order, for the weapon and of the kind
//! ADR-078's table gives: common options at 1, 4 and 7 (Main), 2 and 8 (Sub), and from 11 on
//! Main, Sub, Main in turn; the weapon's three styles at 3 (Main) and 5 (Sub); the chosen Main
//! style's upgrades at 6 and 9 and its capstones at 10. A common offer draws `OFFER_SIZE`
//! candidates from that weapon's common pool, from the run's generator, leaving out the candidates it already holds; with fewer left it holds those, and
//! with none it does not open. Its options are the candidates, then a reroll of each while the
//! run has rerolls (shared with the stars' offers) and another option could come; never a skip
//! or a banish. Taking an option adds its effect as a reward of that kind does, a range option
//! widening the combo steps as a range reward does, and counts it for its weapon.
//!
//! A style, upgrade or capstone offer is not drawn: it shows its fixed choices in the content's
//! order, takes nothing from the generator, and its options are the choices alone: no reroll,
//! skip or banish. Taking one applies its effect as a common option's does; a style is held
//! for the run, and the Main style's upgrades and capstone are kept in level order. An inline
//! weapon has no expertise content, so none of its levels opens, its style's included; and a
//! level 6, 9 or 10 with no Main style chosen opens nothing (fail closed: in level order the
//! style at 3 always comes first for a weapon with content).
//!
//! Past level 10, once neither weapon has a common option left to offer, the levels due are
//! passed over at once, so the offers' cost stays bounded too.

use crate::engine::{AgentSource, ExpertiseGauge, ExpertiseOffer, State};
use crate::event::DomainEvent;
use crate::expertise::{
    Expertise, ExpertiseEffect, ExpertiseOption, TABLE_LEVELS, WeaponExpertise,
};
use crate::id::ContentId;
use crate::rng::SplitMix64;
use crate::snapshot::{
    ExpertiseCandidateView, ExpertiseOfferKind, ExpertiseOfferView, OfferOption, WeaponRole,
};
use crate::stage::{ExpertisePoints, Stage};
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
    let taken = |role| vec![0; pool(stage, expertise, role).len()];
    gives_points(stage).then(|| ExpertiseGauge {
        level: 0,
        points: 0,
        next: need(expertise, 1, 0),
        offered: 0,
        offer: None,
        main_taken: taken(WeaponRole::Main),
        sub_taken: taken(WeaponRole::Sub),
        range_pct: 0,
        main_style: None,
        sub_style: None,
        main_path: Vec::new(),
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

/// The weapon and the kind of the offer level `level` opens (ADR-078 §4); none at level 0.
fn slot(level: u32) -> Option<(WeaponRole, ExpertiseOfferKind)> {
    use ExpertiseOfferKind::{Capstone, Common, Style, Upgrade};
    use WeaponRole::{Main, Sub};
    Some(match level {
        0 => return None,
        1 | 4 | 7 => (Main, Common),
        2 | 8 => (Sub, Common),
        3 => (Main, Style),
        5 => (Sub, Style),
        6 | 9 => (Main, Upgrade),
        10 => (Main, Capstone),
        _ if (level - 11) % 3 == 1 => (Sub, Common),
        _ => (Main, Common),
    })
}

/// The expertise content of the weapon in `role`: none for an inline weapon.
fn content<'a>(
    stage: &Stage,
    expertise: &'a Expertise,
    role: WeaponRole,
) -> Option<&'a WeaponExpertise> {
    let id = match role {
        WeaponRole::Main => match &stage.player.main {
            WeaponRef::Preset { id } => id,
            WeaponRef::Inline(_) => return None,
        },
        WeaponRole::Sub => match &stage.player.sub {
            WeaponRef::Preset { id } => id,
            WeaponRef::Inline(_) => return None,
        },
    };
    expertise.weapon(id)
}

/// The common pool of the weapon in `role`: empty for an inline weapon, which has no expertise
/// content.
fn pool<'a>(stage: &Stage, expertise: &'a Expertise, role: WeaponRole) -> &'a [ExpertiseOption] {
    content(stage, expertise, role).map_or(&[], |w| &w.common)
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

/// The fixed choices of level `level`'s style, upgrade or capstone offer for the weapon in
/// `role`, in the content's order: its styles, or the chosen Main style's upgrades or
/// capstones. Empty with no content, at a common level, or at 6, 9 or 10 with no Main style
/// chosen.
fn fixed<'a>(
    stage: &Stage,
    expertise: &'a Expertise,
    g: &ExpertiseGauge,
    role: WeaponRole,
    level: u32,
) -> Vec<Choice<'a>> {
    let Some(w) = content(stage, expertise, role) else {
        return Vec::new();
    };
    let path = |pick: fn(&'a crate::expertise::Style) -> &'a Vec<ExpertiseOption>| {
        g.main_style
            .and_then(|i| w.styles.get(usize::from(i)))
            .map_or_else(Vec::new, |style| {
                pick(style).iter().map(Choice::from).collect()
            })
    };
    match slot(level) {
        Some((_, ExpertiseOfferKind::Style)) => w
            .styles
            .iter()
            .map(|st| Choice {
                id: &st.id,
                effect: st.effect,
                amount: st.amount,
            })
            .collect(),
        Some((WeaponRole::Main, ExpertiseOfferKind::Upgrade)) if level == 6 => {
            path(|st| &st.upgrade_6)
        }
        Some((WeaponRole::Main, ExpertiseOfferKind::Upgrade)) => path(|st| &st.upgrade_9),
        Some((WeaponRole::Main, ExpertiseOfferKind::Capstone)) => path(|st| &st.capstone),
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
        ExpertiseOfferKind::Common => pool(stage, expertise, o.weapon).get(at).map(Choice::from),
        _ => fixed(stage, expertise, g, o.weapon, o.level)
            .into_iter()
            .nth(at),
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

/// Whether some option besides `held` could still be offered from the weapon in `role`.
fn any_left(stage: &Stage, expertise: &Expertise, role: WeaponRole, held: &[u8]) -> bool {
    let pool = pool(stage, expertise, role);
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
        if g.offered >= TABLE_LEVELS as u32
            && !any_left(stage, expertise, WeaponRole::Main, &[])
            && !any_left(stage, expertise, WeaponRole::Sub, &[])
        {
            // Past the style path, nothing can ever be offered again: every level due is
            // passed over.
            g.offered = g.level;
            return;
        }
        g.offered += 1;
        let level = g.offered;
        let Some((weapon, kind)) = slot(level) else {
            continue;
        };
        let candidates: Vec<u8> = if kind == ExpertiseOfferKind::Common {
            let pool = pool(stage, expertise, weapon);
            let mut candidates = Vec::with_capacity(OFFER_SIZE);
            while candidates.len() < OFFER_SIZE {
                match draw(pool, rng, &candidates) {
                    Some(i) => candidates.push(i),
                    None => break,
                }
            }
            candidates
        } else {
            let n = fixed(stage, expertise, g, weapon, level).len();
            (0..n).map_while(|i| u8::try_from(i).ok()).collect()
        };
        if candidates.is_empty() {
            continue;
        }
        let o = ExpertiseOffer {
            level,
            weapon,
            kind,
            candidates,
        };
        events.push(DomainEvent::ExpertiseOfferOpened {
            level,
            weapon,
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
        && any_left(stage, expertise, o.weapon, &o.candidates)
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
    let pool = pool(stage, expertise, o.weapon);
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
            match (o.kind, o.weapon) {
                (ExpertiseOfferKind::Style, WeaponRole::Main) => g.main_style = Some(place),
                (ExpertiseOfferKind::Style, WeaponRole::Sub) => g.sub_style = Some(place),
                _ => g.main_path.push(place),
            }
            events.push(DomainEvent::ExpertiseOptionTaken {
                weapon: o.weapon,
                option: id,
                taken: 1,
            });
        }
        OfferOption::Take { candidate } => {
            let Some(&i) = o.candidates.get(usize::from(candidate)) else {
                return;
            };
            let at = usize::from(i);
            let taken = match o.weapon {
                WeaponRole::Main => &mut g.main_taken,
                WeaponRole::Sub => &mut g.sub_taken,
            };
            let (Some(n), Some(picked)) = (taken.get_mut(at), pool.get(at)) else {
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
                weapon: o.weapon,
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

/// The IDs of the Main and the Sub style chosen, and of the Main style's path taken, in level
/// order.
pub(crate) fn styles_view(
    stage: &Stage,
    expertise: &Expertise,
    g: &ExpertiseGauge,
) -> (Option<ContentId>, Option<ContentId>, Vec<ContentId>) {
    let style = |role, chosen: Option<u8>| {
        let w = content(stage, expertise, role)?;
        Some(w.styles.get(usize::from(chosen?))?.id.clone())
    };
    let path = [6, 9, 10]
        .into_iter()
        .zip(&g.main_path)
        .filter_map(|(level, i)| {
            fixed(stage, expertise, g, WeaponRole::Main, level)
                .into_iter()
                .nth(usize::from(*i))
                .map(|c| c.id.clone())
        })
        .collect();
    (
        style(WeaponRole::Main, g.main_style),
        style(WeaponRole::Sub, g.sub_style),
        path,
    )
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
        weapon: o.weapon,
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
