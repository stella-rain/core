//! Reading and changing the entities of a run by `Who`: the main character, the boss, a
//! companion or an enemy. Statuses and aggro live here too, since every kind of entity has
//! them.

use crate::engine::{DotState, State, StatusKind, StatusState};
use crate::event::DomainEvent;
use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::rewards::MAX_TYPE_BONUS_PCT;
use crate::rune::Rune;
use crate::stage::Stage;

use super::{MAX_AGGRO, PLAYER_ID, Shot};

/// An entity of the run, by place. Indexes are into `State::companions`, `State::enemies` and
/// the boss's parts, and stay valid within a tick: nothing is removed until the collision step
/// ends (a broken part keeps its place with 0 hit points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Who {
    Player,
    Boss,
    BossPart(usize),
    Companion(usize),
    Enemy(usize),
}

/// A status stacks up to this many times.
const MAX_STACKS: u16 = 5;
/// Bounds on the percent a kind of status can add up to.
const MAX_BONUS_PCT: u32 = 1000;
const MAX_SLOW_PCT: u32 = 90;

impl State {
    fn boss_state(&self) -> &crate::engine::BossState {
        self.boss
            .as_ref()
            .expect("Who::Boss only exists while there is a boss")
    }

    pub(super) fn id_of(&self, who: Who) -> EntityId {
        match who {
            Who::Player => PLAYER_ID,
            Who::Boss => self.boss_state().id,
            Who::BossPart(i) => self.boss_state().parts[i].id,
            Who::Companion(i) => self.companions[i].id,
            Who::Enemy(i) => self.enemies[i].id,
        }
    }

    pub(super) fn at(&self, who: Who) -> Point {
        match who {
            Who::Player => self.player.at,
            Who::Boss => self.boss_state().at,
            // Where the part is: the boss's place and the part's offset (the stage has it).
            Who::BossPart(i) => self.part_at(i),
            Who::Companion(i) => self.companions[i].at,
            Who::Enemy(i) => self.enemies[i].at,
        }
    }

    pub(super) fn hp(&self, who: Who) -> u32 {
        match who {
            Who::Player => self.player.hp,
            Who::Boss => self.boss_state().hp,
            Who::BossPart(i) => self.boss_state().parts[i].hp,
            Who::Companion(i) => self.companions[i].hp,
            Who::Enemy(i) => self.enemies[i].hp,
        }
    }

    pub(super) fn hp_mut(&mut self, who: Who) -> &mut u32 {
        match who {
            Who::Player => &mut self.player.hp,
            Who::Boss => &mut self.boss.as_mut().expect("a boss").hp,
            Who::BossPart(i) => &mut self.boss.as_mut().expect("a boss").parts[i].hp,
            Who::Companion(i) => &mut self.companions[i].hp,
            Who::Enemy(i) => &mut self.enemies[i].hp,
        }
    }

    pub(super) fn defence(&self, who: Who) -> u32 {
        match who {
            Who::Player => self.player.defence,
            Who::Boss => self.boss_state().defence,
            Who::BossPart(i) => self.boss_state().parts[i].defence,
            Who::Companion(i) => self.companions[i].defence,
            Who::Enemy(i) => self.enemies[i].defence,
        }
    }

    pub(super) fn max_hp(&self, who: Who) -> u32 {
        match who {
            Who::Player => self.player.max_hp,
            Who::Boss => self.boss_state().max_hp,
            Who::BossPart(i) => self.boss_state().parts[i].max_hp,
            Who::Companion(i) => self.companions[i].max_hp,
            Who::Enemy(i) => self.enemies[i].max_hp,
        }
    }

    /// Where part `i` of the boss is: the boss's place plus the part's offset.
    fn part_at(&self, i: usize) -> Point {
        let b = self.boss_state();
        let offset = b.parts[i].offset;
        Point {
            x: b.at.x.saturating_add(offset.x),
            y: b.at.y.saturating_add(offset.y),
        }
    }

    /// Aggro belongs to allies; everything else has none.
    pub(super) fn aggro(&self, who: Who) -> u32 {
        match who {
            Who::Player => self.player.aggro,
            Who::Companion(i) => self.companions[i].aggro,
            Who::Boss | Who::BossPart(_) | Who::Enemy(_) => 0,
        }
    }

    pub(super) fn statuses(&self, who: Who) -> &[StatusState] {
        match who {
            Who::Player => &self.player.statuses,
            Who::Boss => &self.boss_state().statuses,
            Who::BossPart(i) => &self.boss_state().parts[i].statuses,
            Who::Companion(i) => &self.companions[i].statuses,
            Who::Enemy(i) => &self.enemies[i].statuses,
        }
    }

    fn statuses_mut(&mut self, who: Who) -> &mut Vec<StatusState> {
        match who {
            Who::Player => &mut self.player.statuses,
            Who::Boss => &mut self.boss.as_mut().expect("a boss").statuses,
            Who::BossPart(i) => &mut self.boss.as_mut().expect("a boss").parts[i].statuses,
            Who::Companion(i) => &mut self.companions[i].statuses,
            Who::Enemy(i) => &mut self.enemies[i].statuses,
        }
    }

    /// The main character and the companions are allies; enemies and the boss are not.
    pub(super) fn is_ally(who: Who) -> bool {
        matches!(who, Who::Player | Who::Companion(_))
    }

    pub(super) fn is_alive(&self, who: Who) -> bool {
        self.hp(who) > 0
    }

    /// The entity with this ID, if it is still around.
    pub(super) fn find(&self, id: EntityId) -> Option<Who> {
        if id == PLAYER_ID {
            return Some(Who::Player);
        }
        if self.boss.as_ref().is_some_and(|b| b.id == id) {
            return Some(Who::Boss);
        }
        if let Some(i) = self
            .boss
            .iter()
            .flat_map(|b| &b.parts)
            .position(|p| p.id == id && p.hp > 0)
        {
            return Some(Who::BossPart(i));
        }
        if let Some(i) = self.companions.iter().position(|c| c.id == id) {
            return Some(Who::Companion(i));
        }
        // Enemies are kept in ascending ID order (`enemies_in_id_order`), so a binary search
        // finds one in about 9 steps among 500.
        self.enemies
            .binary_search_by_key(&id, |e| e.id)
            .ok()
            .map(Who::Enemy)
    }

    /// The ally with this entity ID, if it is still around.
    pub(super) fn find_ally(&self, id: EntityId) -> Option<Who> {
        if id == PLAYER_ID {
            return Some(Who::Player);
        }
        self.companions
            .iter()
            .position(|c| c.id == id)
            .map(Who::Companion)
    }

    /// Adds to an ally's aggro (ADR-036), up to a cap. Others have none to raise.
    pub(super) fn raise_aggro(&mut self, id: EntityId, amount: u32) {
        let slot = match self.find_ally(id) {
            Some(Who::Player) => &mut self.player.aggro,
            Some(Who::Companion(i)) => &mut self.companions[i].aggro,
            _ => return,
        };
        *slot = slot.saturating_add(amount).min(MAX_AGGRO);
    }

    /// Applies a status, or adds a stack and refreshes the duration if it is already there.
    pub(super) fn add_status(
        &mut self,
        target: Who,
        kind: StatusKind,
        source: EntityId,
        value_pct: u16,
        duration: u32,
        events: &mut Vec<DomainEvent>,
    ) {
        let list = self.statuses_mut(target);
        let stacks = match list.iter_mut().find(|s| s.kind == kind) {
            Some(s) => {
                s.stacks = (s.stacks + 1).min(MAX_STACKS);
                s.remaining = s.remaining.max(duration);
                s.source = source;
                s.value_pct = value_pct;
                s.stacks
            }
            None => {
                list.push(StatusState {
                    kind,
                    source,
                    remaining: duration,
                    stacks: 1,
                    value_pct,
                    dot: None,
                });
                1
            }
        };
        events.push(DomainEvent::StatusApplied {
            target: self.id_of(target),
            status: ContentId(kind.name().to_owned()),
            stacks,
            duration_ticks: duration,
        });
    }
}

impl State {
    /// Applies a damage-over-time status of `kind` (ADR-099), or adds a stack to the one there,
    /// up to `max_stacks`: one status of a kind per entity, its stacks shared. Either way its
    /// duration starts again and `dot`, with the multipliers of its source fixed now, replaces
    /// what was there; but its tick timer runs on, so reapplying never puts a tick off.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_dot(
        &mut self,
        target: Who,
        kind: StatusKind,
        source: EntityId,
        mut dot: DotState,
        duration: u32,
        max_stacks: u16,
        events: &mut Vec<DomainEvent>,
    ) {
        let list = self.statuses_mut(target);
        let stacks = match list.iter_mut().find(|s| s.kind == kind) {
            Some(s) => {
                if let Some(old) = s.dot {
                    dot.next_in = old.next_in;
                }
                s.stacks = s.stacks.saturating_add(1).min(max_stacks);
                s.remaining = duration;
                s.source = source;
                s.dot = Some(dot);
                s.stacks
            }
            None => {
                list.push(StatusState {
                    kind,
                    source,
                    remaining: duration,
                    stacks: 1,
                    value_pct: 0,
                    dot: Some(dot),
                });
                1
            }
        };
        events.push(DomainEvent::StatusApplied {
            target: self.id_of(target),
            status: ContentId(kind.name().to_owned()),
            stacks,
            duration_ticks: duration,
        });
    }
}

/// The damage-over-time statuses tick (ADR-099), in step 6 before any hit lands: on the boss,
/// its unbroken parts, then the enemies in entity-ID order. Each counts down to its next tick,
/// and on it deals its damage a stack times its stacks, raised by the multipliers fixed when it
/// was last applied, then as a hit by its source of its type (`damage`): the target's
/// vulnerability, match-up and defence at the tick, no combo step's, so it gives no expertise,
/// fires no rune and applies no status. While the boss is invulnerable, the tick does it and
/// its parts no harm. Then its duration runs down, and at 0 it is gone. One that ends the
/// entity, or finds it already at 0 hit points (a broken part), goes with it.
pub(super) fn tick_dots(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.boss.as_ref().is_some_and(|b| b.hp > 0) {
        tick_dots_of(stage, s, Who::Boss, events);
        let parts = s.boss.as_ref().map_or(0, |b| b.parts.len());
        for i in 0..parts {
            tick_dots_of(stage, s, Who::BossPart(i), events);
        }
    }
    for i in 0..s.enemies.len() {
        tick_dots_of(stage, s, Who::Enemy(i), events);
    }
}

fn tick_dots_of(stage: &Stage, s: &mut State, who: Who, events: &mut Vec<DomainEvent>) {
    if s.statuses(who).iter().all(|st| st.dot.is_none()) {
        return;
    }
    let absorbed = matches!(who, Who::Boss | Who::BossPart(_))
        && s.boss.as_ref().is_some_and(|b| b.invulnerable > 0);
    let mut i = 0;
    while i < s.statuses(who).len() && s.is_alive(who) {
        let st = s.statuses(who)[i];
        let Some(mut dot) = st.dot else {
            i += 1;
            continue;
        };
        dot.next_in = dot.next_in.saturating_sub(1);
        let due = dot.next_in == 0;
        if due {
            dot.next_in = dot.interval.max(1);
        }
        let remaining = st.remaining.saturating_sub(1);
        let list = s.statuses_mut(who);
        list[i].dot = Some(dot);
        list[i].remaining = remaining;
        if due && !absorbed {
            let base = dot.damage.saturating_mul(u32::from(st.stacks));
            let shot = Shot {
                at: s.at(who),
                owner: st.source,
                friendly: true,
                damage: scale_damage(base, dot.atk_up_pct, dot.type_bonus_pct, 0, 0, 0),
                damage_type: dot.damage_type,
                breaks_charge: false,
                pull: None,
                combo: false,
                rune: Rune::of_dot(st.kind),
            };
            super::damage(stage, s, shot, who, events);
        }
        if remaining == 0 {
            s.statuses_mut(who).remove(i);
        } else {
            i += 1;
        }
    }
    if !s.is_alive(who) {
        s.statuses_mut(who).retain(|st| st.dot.is_none());
    }
}

/// Enemies are in ascending entity-ID order, which `State::find` relies on: spawns and summons
/// push new IDs at the end, and removal keeps the order. Checked after every tick in debug builds.
pub(super) fn enemies_in_id_order(s: &State) -> bool {
    s.enemies.windows(2).all(|w| w[0].id < w[1].id)
}

/// The percent that the statuses of `kind` add up to (value times stacks), within its cap.
pub(super) fn status_pct(statuses: &[StatusState], kind: StatusKind) -> u32 {
    let sum: u32 = statuses
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| u32::from(s.value_pct) * u32::from(s.stacks))
        .sum();
    sum.min(if kind == StatusKind::Slow {
        MAX_SLOW_PCT
    } else {
        MAX_BONUS_PCT
    })
}

/// Percent of its speed an entity keeps, after slowing.
pub(super) fn speed_pct(statuses: &[StatusState]) -> i64 {
    100 - i64::from(status_pct(statuses, StatusKind::Slow))
}

/// `K` of ADR-056: a defence of `K` halves a hit. A tuning value.
const DEFENCE_K: u64 = 100;

/// `base` damage, raised by the attacker's atk-up and type bonus (ADR-100) and by the target's
/// vulnerability (percent each), changed by the hit's match-up (`match_up_pct`, -100 to 100;
/// ADR-100), then reduced to `K / (K + defence)` of it (ADR-056), in that order, truncating
/// toward zero at each step in 64 bits. A hit with any damage at all does at least 1.
pub(super) fn scale_damage(
    base: u32,
    atk_up_pct: u32,
    type_bonus_pct: u32,
    vulnerable_pct: u32,
    match_up_pct: i64,
    defence: u32,
) -> u32 {
    // Callers hold atk-up and vulnerability to `MAX_BONUS_PCT` and the type bonus to
    // `MAX_TYPE_BONUS_PCT`; the clamp keeps 64 bits enough whatever reaches here.
    let type_bonus = u64::from(type_bonus_pct.min(MAX_TYPE_BONUS_PCT));
    let raised = u64::from(base) * u64::from(100 + atk_up_pct) / 100 * (100 + type_bonus) / 100
        * u64::from(100 + vulnerable_pct)
        / 100;
    // The table holds the percent to -100 and 100 (`MAX_MATCH_UP_PCT`); the clamp keeps the
    // factor from 0 to 200 whatever reaches here.
    let factor = (100 + match_up_pct.clamp(-100, 100)).unsigned_abs();
    let scaled = raised * factor / 100 * DEFENCE_K / (DEFENCE_K + u64::from(defence));
    u32::try_from(scaled)
        .unwrap_or(u32::MAX)
        .max(u32::from(base > 0))
}

/// Statuses run out and aggro fades, once per tick. Damage-over-time statuses run out as they
/// tick instead (`tick_dots`); whether there is one on anyone is returned, so a tick without
/// one skips that pass.
pub(super) fn tick_statuses_and_aggro(s: &mut State) -> bool {
    fn age(list: &mut Vec<StatusState>) -> bool {
        let mut dots = false;
        for st in list.iter_mut() {
            if st.dot.is_some() {
                dots = true;
            } else {
                st.remaining = st.remaining.saturating_sub(1);
            }
        }
        list.retain(|st| st.dot.is_some() || st.remaining > 0);
        dots
    }
    let mut dots = age(&mut s.player.statuses);
    s.player.aggro = s.player.aggro.saturating_sub(super::AGGRO_DECAY);
    if let Some(b) = &mut s.boss {
        dots |= age(&mut b.statuses);
        for part in &mut b.parts {
            dots |= age(&mut part.statuses);
        }
    }
    for c in &mut s.companions {
        dots |= age(&mut c.statuses);
        c.aggro = c.aggro.saturating_sub(super::AGGRO_DECAY);
    }
    for e in &mut s.enemies {
        dots |= age(&mut e.statuses);
    }
    dots
}

#[cfg(test)]
mod tests {
    use super::scale_damage;
    use crate::rewards::MAX_TYPE_BONUS_PCT;

    #[test]
    fn a_match_up_scales_after_vulnerability_and_before_defence() {
        assert_eq!(scale_damage(10, 0, 0, 0, 20, 0), 12);
        assert_eq!(scale_damage(10, 0, 0, 0, -20, 0), 8);
        assert_eq!(scale_damage(10, 0, 0, 50, 20, 100), 9);
        assert_eq!(scale_damage(10, 0, 0, 0, 0, 0), 10);
    }

    #[test]
    fn damage_saturates_and_never_falls_below_1() {
        // Every multiplier at its cap, weak at the table's cap: saturated to 32 bits.
        assert_eq!(scale_damage(u32::MAX, 1000, 0, 1000, 100, 0), u32::MAX);
        // A match-up past the table's cap is held to it.
        assert_eq!(scale_damage(10, 0, 0, 0, i64::MAX, 0), 20);
        assert_eq!(scale_damage(10, 0, 0, 0, i64::MIN, 0), 1);
        assert_eq!(scale_damage(10, 0, 0, 0, -100, 0), 1);
        assert_eq!(scale_damage(0, 0, 0, 0, 100, 0), 0);
        // The type bonus at its cap and past it, with everything else at its cap too.
        assert_eq!(scale_damage(u32::MAX, 1000, 1000, 1000, 100, 0), u32::MAX);
        assert_eq!(scale_damage(10, 0, u32::MAX, 0, 0, 0), 110);
    }

    #[test]
    fn a_type_bonus_raises_with_the_atk_up_and_multiplies_with_the_match_up() {
        // ADR-100's example: Pyro +50 against a target strong against Pyro, 1.5 × 0.8.
        assert_eq!(scale_damage(100, 0, 50, 0, -20, 0), 120);
        // After the atk-up, truncated at each step: 7 × 1.5 = 10, × 1.5 = 15.
        assert_eq!(scale_damage(7, 50, 50, 0, 0, 0), 15);
        assert_eq!(scale_damage(10, 0, 0, 0, 0, 0), 10);
    }

    #[test]
    fn the_largest_hit_the_bounds_allow_stays_below_the_32_bit_limit() {
        // ADR-103: the damage scale makes the limit of ADR-056 decision 5 100 times nearer, so
        // the largest base damage the validator allows, with the atk-up, the type bonus and the
        // vulnerability at their caps and the match-up at its cap, is held below it: that hit
        // is dealt whole, not saturated.
        use crate::validate::limits::MAX_DAMAGE;
        assert_eq!(MAX_DAMAGE, 1_000_000, "10,000 at the design values, x100");
        let hit = scale_damage(
            MAX_DAMAGE,
            super::MAX_BONUS_PCT,
            MAX_TYPE_BONUS_PCT,
            super::MAX_BONUS_PCT,
            100,
            0,
        );
        let exact = u64::from(MAX_DAMAGE) * 11 * 11 * 11 * 2;
        assert_eq!(u64::from(hit), exact);
        assert!(exact < u64::from(u32::MAX), "{exact} of {}", u32::MAX);
        // The same hit twice over would saturate: the margin is under a factor of 2.
        assert!(exact * 2 > u64::from(u32::MAX));
    }
}
