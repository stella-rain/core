//! The style grants: the grant types, their compilers and their folds.

use super::PATH_PLACES;

use crate::expertise::CounterEffect;
use crate::expertise::Gauge;
use crate::expertise::MAX_ORB_BREAK_PCT;
use crate::expertise::MAX_SPIRIT_BLEED_STACKS;
use crate::expertise::SpiritEffect;
use crate::expertise::StyleKind;
use crate::expertise::TempoEffect;
use crate::expertise::TriggerEffect;
use crate::fixed::Fx;
use crate::limits::MAX_ADVANCE_DISTANCE;
use crate::limits::MAX_COUNTER_WINDOW_TICKS;
use crate::limits::MAX_GAUGE_DAMAGE_PCT;
use crate::limits::MAX_PCT;
use crate::limits::MAX_SPIRIT_DECAY_TICKS;
use crate::limits::MAX_SPIRIT_LEVEL;
use crate::limits::MAX_TEMPO_STACKS;
use crate::limits::MAX_TRIGGER_TICKS;
use crate::limits::MAX_WEAPON_GAUGE;
use crate::run_content::RunContent;
use crate::run_stats::MAX_ATTACK_SPEED_PCT;
use crate::weapon::RepeatBleed;
use crate::weapon::Weapon;
use crate::weapon::WeaponRef;

/// A style's orb break resolved once: the chance in percent and the radius round what a repeat
/// strikes, inside their caps (ADR-SVC-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbBreakGrant {
    pub chance_pct: u8,
    pub radius: Fx,
}

/// A style's breaker resolved once: a factor in percent on the stagger fills, on the poise damage
/// of the combo hits and on a smash's damage on a boss part (each 100: none), and the burst round
/// a target the player staggers, every number inside its cap (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BreakerGrant {
    pub stagger_pct: u32,
    pub poise_pct: u32,
    pub part_break_pct: u32,
    /// A factor on the ticks of a stagger the player causes (100: none).
    pub stagger_ticks_pct: u32,
    pub burst: Option<BurstGrant>,
}

/// A stagger burst resolved once: its radius, the damage a percent of the staggering hit's (0:
/// none), a stun in ticks (0: none) and whether it knocks down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BurstGrant {
    pub radius: i32,
    pub damage_pct: u32,
    pub stun_ticks: u32,
    pub knock_down: bool,
}

/// A style's giant trigger resolved once: how long the state runs and each effect folded into its
/// field (0 where it has none, 100 for the rally factor), every number inside its cap (ADR-SVC-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TriggerGrant {
    pub ticks: u16,
    pub range_pct: u32,
    pub smash_pct: u32,
    pub rally_pct: u32,
    pub super_armour: bool,
    pub hold_move_pct: u32,
    /// The trigger names `end_ring`: its end plays the ring `World::ring_of` holds.
    pub end_ring: bool,
}

/// A style's combo tempo resolved once: the most stacks, each stack's attack speed and whether a
/// full-stack smash keeps them, every number inside its cap (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TempoGrant {
    pub max_stacks: u8,
    pub speed_pct: u32,
    pub keep_on_smash: bool,
    /// Full stacks in a held hold are super armour, and only that armour shelters the stacks.
    pub hold_armour: bool,
    /// Percent added to the range of every attack the combo starts while the stacks are full, 0
    /// where no level names it.
    pub range_pct_at_full: u32,
    /// A combo that ends on a full-stack smash leaves the stacks at a reset that is not the window
    /// running out (`player::tempo_carries`).
    pub carry_at_full: bool,
}

/// A style's counter bonus resolved once: the window and each effect folded into its field, 0
/// where it has none (100 for the stagger factor), every number inside its cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CounterGrant {
    pub window: u16,
    pub damage_pct: u32,
    pub stagger_pct: u32,
    pub stun_ticks: u32,
    pub advance: Fx,
    /// Give back one spent evade charge at the counter's first landed hit.
    pub refund_evade: bool,
    /// Percent of the weapon gauge's bar filled at that hit, 0 for none.
    pub gauge_fill_pct: u32,
}

/// A style's spirit resolved once: the countdown and, for each level, the effects folded into
/// one flat entry, every number clamped to its cap again (ADR-SVC-1, ADR-SIM-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpiritGrant {
    /// Ticks a level lasts without a smash.
    pub decay_ticks: u16,
    /// Entry `i` is level `i + 1`'s own effects.
    pub levels: Vec<SpiritFx>,
}

/// One spirit level's effects, 0 where it has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpiritFx {
    /// Added to the attack-up percent of a combo step.
    pub damage_pct: u32,
    /// Added to the range percent of a combo step.
    pub range_pct: u32,
    /// Bleed stacks a smash hit adds on top of its own.
    pub bleed_stacks: u32,
}

impl SpiritGrant {
    /// The effects of `level` (1 to the top), none for level 0.
    pub fn fx(&self, level: u8) -> Option<&SpiritFx> {
        self.levels.get(usize::from(level.checked_sub(1)?))
    }
}

/// The copies of `base`, the run's weapon, for each style of the weapon's expertise that grants
/// a kind: one per number of places of its path taken (0 to 3). Empty when the weapon is not a
/// preset with expertise content or no style grants anything.
pub(super) fn grant_weapons(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<Weapon>> {
    let (WeaponRef::Preset { id, .. }, Some(base)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let grants = |s: &crate::expertise::Style| {
        !s.kinds.is_empty()
            || crate::expertise::style_levels(s)
                .into_iter()
                .any(|(_, o)| o.is_some_and(|o| !o.kinds.is_empty()))
    };
    if !expertise.styles.iter().any(grants) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(
                grants(style).then(|| crate::expertise::weapon_with_style(base, style, taken)),
            );
        }
    }
    out
}

/// Adds each distinct repeat-bleed row of the tree to `rows`, in the order first met.
pub(super) fn collect_bleeds(branches: &crate::weapon::Branches, rows: &mut Vec<RepeatBleed>) {
    for step in [false, true]
        .into_iter()
        .filter_map(|smash| branches.get(smash))
    {
        for bleed in step.repeats.iter().filter_map(|r| r.bleed) {
            if !rows.contains(&bleed) {
                rows.push(bleed);
            }
        }
        collect_bleeds(&step.next, rows);
    }
}

/// Whether `pred` holds for some step of the tree under `branches`.
pub(super) fn any_step(
    branches: &crate::weapon::Branches,
    pred: &dyn Fn(&crate::weapon::Step) -> bool,
) -> bool {
    [false, true]
        .into_iter()
        .filter_map(|smash| branches.get(smash))
        .any(|step| pred(step) || any_step(&step.next, pred))
}

/// The gauge of each style and number of places of its path taken (the same index as
/// `grant_weapons`), numbers clamped to their caps again (ADR-SVC-1). Empty when no style of the
/// weapon names one.
pub(super) fn grant_gauges(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<Gauge>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let gauge_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        std::iter::once(s.kinds.as_slice())
            .chain(places)
            .flatten()
            .find_map(|k| match k {
                StyleKind::Gauge(g) => Some(clamped(g)),
                _ => None,
            })
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(gauge_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The kinds of a style's own level, then of each of the first `taken` places of its path, in
/// path order (ADR-SIM-9 point 3).
fn path_kinds(s: &crate::expertise::Style, taken: usize) -> impl Iterator<Item = &StyleKind> {
    let places = crate::expertise::style_levels(s)
        .into_iter()
        .take(taken)
        .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
    std::iter::once(s.kinds.as_slice()).chain(places).flatten()
}

/// The spirit of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `spirit` kinds of the style's own level, then of each place taken, in
/// path order, a later level's countdown and level effects replacing the earlier ones (see
/// `spirit_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_spirits(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<SpiritGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let spirit_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<SpiritGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::Spirit(sp) = k {
                spirit_fold(grant.get_or_insert_with(SpiritGrant::none), sp);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(spirit_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The counter bonus of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `counter_bonus` kinds of the style's own level, then of each place
/// taken, in path order, a later level's window and effects replacing the earlier ones (see
/// `counter_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_counters(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<CounterGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let counter_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<CounterGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::CounterBonus(c) = k {
                counter_fold(grant.get_or_insert_with(CounterGrant::none), c);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(counter_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The combo tempo of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `combo_tempo_pct` kinds of the style's own level, then of each place
/// taken, in path order, a later level's cap and effects replacing the earlier ones (see
/// `tempo_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_tempos(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<TempoGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let tempo_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<TempoGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::ComboTempoPct(t) = k {
                tempo_fold(grant.get_or_insert_with(TempoGrant::none), t);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(tempo_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The giant trigger of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `giant_trigger` kinds of the style's own level, then of each place
/// taken, in path order, a later level's length and effects replacing the earlier ones (see
/// `trigger_fold`), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_triggers(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<TriggerGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let trigger_of = |s: &crate::expertise::Style, taken: usize| {
        let mut grant: Option<TriggerGrant> = None;
        for k in path_kinds(s, taken) {
            if let StyleKind::GiantTrigger(t) = k {
                trigger_fold(grant.get_or_insert_with(TriggerGrant::none), t);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(trigger_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The breaker of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `breaker` kinds of the style's own level, then of each place taken, in
/// path order, each effect folded into its field and a later level's same effect replacing the
/// earlier one (ADR-SIM-9), clamped (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_breakers(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<BreakerGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let breaker_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        let mut grant: Option<BreakerGrant> = None;
        for kind in std::iter::once(s.kinds.as_slice()).chain(places).flatten() {
            if let StyleKind::Breaker(b) = kind {
                breaker_folded(grant.get_or_insert(BreakerGrant::NONE), b);
            }
        }
        grant
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(breaker_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// The orb break of each style and number of places of its path taken (the same index as
/// `grant_weapons`): the `repeat_breaks_orbs` kind of the style's own level or of a place taken
/// (one per path), clamped again (ADR-SVC-1). Empty when no style of the weapon names one.
pub(super) fn grant_repeat_breaks(
    main: &WeaponRef<Weapon>,
    base: Option<&Weapon>,
    content: &RunContent,
) -> Vec<Option<OrbBreakGrant>> {
    let (WeaponRef::Preset { id, .. }, Some(_)) = (main, base) else {
        return Vec::new();
    };
    let Some(expertise) = content.expertise().weapon(id) else {
        return Vec::new();
    };
    let grant_of = |s: &crate::expertise::Style, taken: usize| {
        let places = crate::expertise::style_levels(s)
            .into_iter()
            .take(taken)
            .filter_map(|(_, o)| o.map(|o| o.kinds.as_slice()));
        std::iter::once(s.kinds.as_slice())
            .chain(places)
            .flatten()
            .find_map(|kind| match kind {
                StyleKind::RepeatBreaksOrbs {
                    chance_pct,
                    radius_px,
                } => Some(OrbBreakGrant {
                    chance_pct: u8::try_from((*chance_pct).clamp(1, MAX_ORB_BREAK_PCT))
                        .unwrap_or(100),
                    radius: Fx::px(
                        i32::try_from((*radius_px).clamp(1, crate::rewards::MAX_BURST_RADIUS_PX))
                            .unwrap_or(1),
                    ),
                }),
                _ => None,
            })
    };
    let mut out = Vec::new();
    for style in &expertise.styles {
        for taken in 0..PATH_PLACES {
            out.push(grant_of(style, taken));
        }
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

impl BreakerGrant {
    /// A breaker that does nothing: every factor 100, no burst.
    const NONE: BreakerGrant = BreakerGrant {
        stagger_pct: 100,
        poise_pct: 100,
        part_break_pct: 100,
        stagger_ticks_pct: 100,
        burst: None,
    };
}

/// `b`'s effects laid over `g`, every number inside its cap.
fn breaker_folded(g: &mut BreakerGrant, b: &crate::expertise::Breaker) {
    use crate::expertise::{BreakerEffect, BurstEffect};
    for e in b.effects.iter().take(5) {
        match e {
            BreakerEffect::StaggerPct { pct } => g.stagger_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::PoisePct { pct } => g.poise_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::PartBreakPct { pct } => g.part_break_pct = (*pct).clamp(100, MAX_PCT),
            BreakerEffect::StaggerTicksPct { pct } => {
                g.stagger_ticks_pct = (*pct).clamp(100, MAX_PCT);
            }
            BreakerEffect::OnStaggerBurst { radius_px, effects } => {
                let radius = (*radius_px).clamp(1, crate::rewards::MAX_BURST_RADIUS_PX);
                let mut burst = BurstGrant {
                    radius: i32::try_from(i64::from(radius) * i64::from(Fx::px(1).0))
                        .unwrap_or(i32::MAX),
                    damage_pct: 0,
                    stun_ticks: 0,
                    knock_down: false,
                };
                for effect in effects.iter().take(3) {
                    match effect {
                        BurstEffect::DamagePct { pct } => {
                            burst.damage_pct = (*pct).clamp(1, MAX_PCT)
                        }
                        BurstEffect::StunTicks { ticks } => {
                            burst.stun_ticks =
                                (*ticks).clamp(1, crate::validate::limits::MAX_STUN_TICKS);
                        }
                        BurstEffect::KnockDown {} => burst.knock_down = true,
                    }
                }
                g.burst = Some(burst);
            }
        }
    }
}

impl TriggerGrant {
    /// A giant trigger that does nothing yet: the longest length, no effect.
    pub(in crate::sim) fn none() -> TriggerGrant {
        TriggerGrant {
            ticks: MAX_TRIGGER_TICKS,
            range_pct: 0,
            smash_pct: 0,
            rally_pct: 100,
            super_armour: false,
            hold_move_pct: 0,
            end_ring: false,
        }
    }
}

/// `t` laid over `g`, every number inside its cap: its length replaces the earlier one and each
/// effect it lists replaces the same effect; an effect it does not list stays, and the armour is
/// true once any level names it.
pub(in crate::sim) fn trigger_fold(g: &mut TriggerGrant, t: &crate::expertise::GiantTrigger) {
    g.ticks = u16::try_from(t.ticks)
        .unwrap_or(MAX_TRIGGER_TICKS)
        .clamp(1, MAX_TRIGGER_TICKS);
    for e in t.effects.iter().take(crate::expertise::MAX_TRIGGER_EFFECTS) {
        match e {
            TriggerEffect::HoldMovePct { pct } => {
                g.hold_move_pct = (*pct).min(crate::expertise::MAX_HOLD_MOVE_PCT);
            }
            TriggerEffect::RangePct { pct } => {
                g.range_pct = (*pct).min(crate::rewards::MAX_RANGE_PCT);
            }
            TriggerEffect::SmashPct { pct } => g.smash_pct = (*pct).min(MAX_PCT),
            TriggerEffect::SuperArmour {} => g.super_armour = true,
            // The ring itself is derived at world build (the compiled smash's first active
            // phase, `compile_rings`); the grant keeps only that it is named.
            TriggerEffect::EndRing {} => g.end_ring = true,
            TriggerEffect::RallyPct { pct } => g.rally_pct = (*pct).clamp(100, MAX_PCT),
        }
    }
}

impl TempoGrant {
    /// A combo tempo that does nothing yet: the most stacks, no speed, no flag.
    pub(in crate::sim) fn none() -> TempoGrant {
        TempoGrant {
            max_stacks: MAX_TEMPO_STACKS,
            speed_pct: 0,
            keep_on_smash: false,
            hold_armour: false,
            range_pct_at_full: 0,
            carry_at_full: false,
        }
    }
}

/// `t` laid over `g`, every number inside its cap: its `max_stacks` replaces the earlier one,
/// `speed_pct` replaces the earlier speed and the flags are true once any level names them.
pub(in crate::sim) fn tempo_fold(g: &mut TempoGrant, t: &crate::expertise::ComboTempo) {
    g.max_stacks = u8::try_from(t.max_stacks)
        .unwrap_or(MAX_TEMPO_STACKS)
        .clamp(1, MAX_TEMPO_STACKS);
    for e in t.effects.iter().take(crate::expertise::MAX_TEMPO_EFFECTS) {
        match e {
            TempoEffect::SpeedPct { pct } => g.speed_pct = (*pct).min(MAX_ATTACK_SPEED_PCT),
            TempoEffect::KeepOnSmash {} => g.keep_on_smash = true,
            TempoEffect::SuperArmourAtFull {} => g.hold_armour = true,
            TempoEffect::RangePctAtFull { pct } => {
                g.range_pct_at_full = (*pct).min(crate::rewards::MAX_RANGE_PCT)
            }
            // Read by `player::tempo_carries`.
            TempoEffect::CarryAtFull {} => g.carry_at_full = true,
        }
    }
}

impl CounterGrant {
    /// A counter bonus that does nothing yet: the longest window, no effect.
    pub(in crate::sim) fn none() -> CounterGrant {
        CounterGrant {
            window: MAX_COUNTER_WINDOW_TICKS,
            damage_pct: 0,
            stagger_pct: 100,
            stun_ticks: 0,
            advance: Fx(0),
            refund_evade: false,
            gauge_fill_pct: 0,
        }
    }
}

/// `c` laid over `g`, every number inside its cap: its window replaces the earlier one and each
/// effect it lists replaces the same effect; an effect it does not list stays.
pub(in crate::sim) fn counter_fold(g: &mut CounterGrant, c: &crate::expertise::CounterBonus) {
    g.window = u16::try_from(c.window_ticks)
        .unwrap_or(MAX_COUNTER_WINDOW_TICKS)
        .clamp(1, MAX_COUNTER_WINDOW_TICKS);
    for e in c.effects.iter().take(6) {
        match e {
            CounterEffect::DamagePct { pct } => g.damage_pct = (*pct).min(MAX_PCT),
            CounterEffect::StaggerPct { pct } => g.stagger_pct = (*pct).clamp(100, MAX_PCT),
            CounterEffect::StunTicks { ticks } => {
                g.stun_ticks = (*ticks).min(crate::validate::limits::MAX_STUN_TICKS);
            }
            CounterEffect::Advance { distance } => {
                g.advance = Fx(distance.0.clamp(0, Fx::px(MAX_ADVANCE_DISTANCE).0));
            }
            CounterEffect::RefundEvade {} => g.refund_evade = true,
            CounterEffect::GaugeFillPct { pct } => g.gauge_fill_pct = (*pct).min(100),
        }
    }
}

impl SpiritGrant {
    /// A spirit that does nothing yet: the longest countdown and no level.
    pub(in crate::sim) fn none() -> SpiritGrant {
        SpiritGrant {
            decay_ticks: MAX_SPIRIT_DECAY_TICKS,
            levels: Vec::new(),
        }
    }
}

/// `sp` laid over `g`, every number inside its cap: its countdown replaces the earlier one and
/// each effect of entry `i` replaces the same effect of level `i + 1`, an entry past the table
/// extending it up to `MAX_SPIRIT_LEVEL`; an empty entry changes nothing.
pub(in crate::sim) fn spirit_fold(g: &mut SpiritGrant, sp: &crate::expertise::Spirit) {
    g.decay_ticks = u16::try_from(sp.decay_ticks)
        .unwrap_or(MAX_SPIRIT_DECAY_TICKS)
        .clamp(1, MAX_SPIRIT_DECAY_TICKS);
    for (i, effects) in sp
        .levels
        .iter()
        .take(usize::from(MAX_SPIRIT_LEVEL))
        .enumerate()
    {
        if g.levels.len() <= i {
            g.levels.push(SpiritFx::default());
        }
        let fx = &mut g.levels[i];
        for e in effects {
            match e {
                SpiritEffect::DamagePct { pct } => fx.damage_pct = (*pct).min(MAX_PCT),
                SpiritEffect::RangePct { pct } => {
                    fx.range_pct = (*pct).min(crate::rewards::MAX_RANGE_PCT);
                }
                SpiritEffect::SmashBleedStacks { stacks } => {
                    fx.bleed_stacks = (*stacks).min(MAX_SPIRIT_BLEED_STACKS);
                }
            }
        }
    }
}

/// `g` with every number inside its cap.
fn clamped(g: &Gauge) -> Gauge {
    let max = g.max.clamp(1, MAX_WEAPON_GAUGE);
    Gauge {
        max,
        per_step: g.per_step.min(max),
        per_hit: g.per_hit.min(max),
        from_damage_pct: g.from_damage_pct.min(MAX_GAUGE_DAMAGE_PCT),
        step_cap: g.step_cap.clamp(1, max),
        spend: g.spend.clone(),
        per_rune: g.per_rune.min(max),
        outer_half_pct: g.outer_half_pct.min(MAX_PCT),
        per_smash_hit: g.per_smash_hit.min(max),
        smash_cap: g.smash_cap.min(max),
    }
}
