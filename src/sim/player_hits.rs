//! Hits on the players of the live rules: damage, down, revive and auras.

use super::entity::{Who, scale_damage, status_pct};
use super::{Blame, Culprit, Shot, World, damage, overlaps, player, skill};
use crate::event::{DomainEvent, HitSource};
use crate::fixed::Point;
use crate::input::Input;
use crate::sim::state::{MAX_PLAYERS, PlayerIdx};
use crate::sim::state::{OUT_TIMER_TICKS, REVIVE_HOLD_TICKS, REVIVE_HP_PCT, REVIVE_RANGE, State};

/// Damage to the main character, raised by its vulnerability and reduced by its defence; it is
/// then invulnerable for a while.
pub(super) fn hurt_player(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    b: Shot,
    by: Blame,
    events: &mut Vec<DomainEvent>,
) {
    let vulnerable = status_pct(
        &s.players[me].statuses,
        crate::sim::state::StatusKind::Vulnerable,
    );
    // The main character has no profile: every hit on it is neutral.
    let dealt = scale_damage(
        b.damage,
        0,
        0,
        vulnerable,
        0,
        s.players[me].defence,
        stage.tuning.defence_k,
    );
    let dealt = skill::absorb(s, me, dealt);
    let party_len = s.players.len();
    let p = &mut s.players[me];
    let before_hp = p.hp;
    p.hp = p.hp.saturating_sub(dealt);
    p.invulnerable = stage.tuning.hit_invulnerable_ticks;
    // sim_version >= 2: a ranged start is kept off a player just hit (the grace, a rules row).
    if stage.tuning.ranged_grace_ticks > 0 {
        p.combat.ranged_grace = stage.tuning.ranged_grace_ticks;
    }
    events.push(DomainEvent::PlayerHit {
        hp_left: p.hp,
        source: HitSource {
            kind: by.kind,
            owner: b.owner,
            attack: match by.by {
                Culprit::Run(a) => Some(a),
                Culprit::Program(p) => stage.attack_of(p),
            }
            .and_then(|a| stage.attacks[usize::from(a.0)].preset_id.clone()),
        },
    });
    // A hit taken ends the combo tempo's stacks, whatever other armour the player has; only the
    // tempo's own armour (full stacks in a hold) shelters them (ADR-SIM-9).
    if !player::tempo_armoured(stage, s, me) {
        player::tempo_lose(stage, s, me, events);
    }
    let p = &mut s.players[me];
    if before_hp > 0 && p.hp == 0 && !p.out && party_len > 1 {
        p.down_ticks = 0;
        p.revive_hold = 0;
        events.push(DomainEvent::PlayerDowned { player: me });
        // A dash, charge, hold or combo step does not wait for the revive.
        player::stand_down(stage, s, me, events);
    }
    if stage.has_gauge {
        player::gauge_from_damage(stage, s, me, before_hp - s.players[me].hp, events);
    }
    if stage.tuning.rally.is_some() {
        player::rally_from_hit(stage, s, me, before_hp - s.players[me].hp, events);
    }
    // A powerful attack breaks a charge (core#83).
    if b.breaks_charge && !player::super_armour(stage, s, me) {
        player::break_charge(stage, s, me, events);
    }
}

/// `me` is up, holds interact and stands within `REVIVE_RANGE` of a downed teammate (the test the
/// reviver search uses): it is rooted for this tick's update (ADR-SIM-7). Every such player is
/// rooted, not only the one whose hold counts. A party of one never calls it.
pub(super) fn near_downed(
    s: &State,
    inputs: &[Input; MAX_PLAYERS as usize],
    me: PlayerIdx,
) -> bool {
    s.players[me].is_up()
        && inputs[me.get()].interact
        && s.players.indices().any(|other| {
            other != me
                && s.players[other].is_downed()
                && overlaps(s.players[me].at, REVIVE_RANGE.0, s.players[other].at, 0)
        })
}

/// Advances down timer and checks teammate revive hold for downed party players (ADR-SIM-7).
pub(crate) fn tick_down_and_revive(
    stage: &World,
    s: &mut State,
    inputs: &[Input; MAX_PLAYERS as usize],
    events: &mut Vec<DomainEvent>,
) {
    if s.players.len() <= 1 {
        return;
    }
    let slots: Vec<PlayerIdx> = s.players.indices().collect();
    for me in slots {
        if !s.players[me].is_downed() {
            continue;
        }
        s.players[me].down_ticks = s.players[me].down_ticks.saturating_add(1);
        if s.players[me].down_ticks >= OUT_TIMER_TICKS {
            s.players[me].out = true;
            s.players[me].revive_hold = 0;
            events.push(DomainEvent::PlayerOut { player: me });
            continue;
        }

        let me_at = s.players[me].at;
        let mut reviver = None;
        for other in s.players.indices() {
            if other != me
                && s.players[other].is_up()
                && inputs[other.get()].interact
                && overlaps(s.players[other].at, REVIVE_RANGE.0, me_at, 0)
            {
                reviver = Some(other);
                break;
            }
        }

        if let Some(reviver) = reviver {
            s.players[me].revive_hold = s.players[me].revive_hold.saturating_add(1);
            if s.players[me].revive_hold >= REVIVE_HOLD_TICKS {
                s.players[me].hp = (s.players[me].max_hp * REVIVE_HP_PCT / 100).max(1);
                s.players[me].invulnerable = stage.tuning.hit_invulnerable_ticks;
                s.players[me].down_ticks = 0;
                s.players[me].revive_hold = 0;
                events.push(DomainEvent::PlayerRevived {
                    player: me,
                    reviver,
                });
            }
        } else {
            s.players[me].revive_hold = 0;
        }
    }
}

/// `b` with the damage a pack leader's aura adds, for a hit of an enemy's on an ally:
/// if the enemy that dealt it is still on the map and not an elite, the highest `damage_percent`
/// among the auras whose leader's centre is within `radius` of its centre (the edge counts)
/// raises the damage to `damage * (100 + percent) / 100`, rounded down. Every leader still in
/// the enemies counts, in index order, so a leader killed in this tick's collisions still gives
/// its aura through the rest of the tick, and none from the next, when the dead are removed.
/// An enemy that has since left the map or died earlier gets none. No random number is drawn.
pub(super) fn aided(stage: &World, s: &State, b: Shot) -> Shot {
    let Ok(at) = s.enemies.binary_search_by_key(&b.owner, |e| e.id) else {
        return b;
    };
    let owner = &s.enemies[at];
    if owner.elite.is_some() {
        return b;
    }
    let percent = aura_percent(aura_sources(stage, s), owner.at);
    Shot {
        damage: u32::try_from(u64::from(b.damage) * (100 + percent) / 100).unwrap_or(u32::MAX),
        ..b
    }
}

/// The auras of the elites on the map, each with its leader's centre, in index order.
pub(crate) fn aura_sources<'a>(
    stage: &'a World,
    s: &'a State,
) -> impl Iterator<Item = (Point, crate::behaviour::Aura)> + 'a {
    s.enemies
        .iter()
        .filter(|l| l.elite.is_some())
        .filter_map(|l| {
            stage
                .defs
                .agent(l.def)
                .elite
                .as_ref()?
                .aura
                .map(|aura| (l.at, aura))
        })
}

/// The highest `damage_percent` among the auras whose leader's centre is within its radius of
/// `at` (the edge counts); 0 when none reaches it.
pub(crate) fn aura_percent(
    auras: impl Iterator<Item = (Point, crate::behaviour::Aura)>,
    at: Point,
) -> u64 {
    auras
        .filter(|(from, aura)| {
            let dx = i64::from(from.x.0) - i64::from(at.x.0);
            let dy = i64::from(from.y.0) - i64::from(at.y.0);
            let reach = i64::from(aura.radius.0);
            dx * dx + dy * dy <= reach * reach
        })
        .map(|(_, aura)| u64::from(aura.damage_percent))
        .max()
        .unwrap_or(0)
}

/// An orb against the players in slot order, unless one is invulnerable: the first it overlaps
/// takes it, and it is used up.
pub(super) fn hit_players_side(
    stage: &World,
    s: &mut State,
    b: Shot,
    by: Blame,
    events: &mut Vec<DomainEvent>,
) -> bool {
    for me in s.players.indices() {
        let p = &s.players[me];
        let touches = p.hp > 0
            && overlaps(
                b.at,
                stage.tuning.orb_radius,
                p.at,
                stage.tuning.player_radius,
            );
        if p.invulnerable > 0 {
            if touches && p.combat.perfect_window > 0 {
                super::player::near_miss(stage, s, me, events);
            }
        } else if touches {
            hurt_player(stage, s, me, aided(stage, s, b), by, events);
            return true;
        }
    }
    false
}

/// An orb against the companions in ID order: the first it overlaps takes it.
pub(super) fn hit_companions_side(
    stage: &World,
    s: &mut State,
    b: Shot,
    events: &mut Vec<DomainEvent>,
) -> bool {
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let radius = stage.defs.agent(c.def).radius.0;
        if c.hp > 0 && overlaps(b.at, stage.tuning.orb_radius, c.at, radius) {
            // An enemy's hit on a companion reads no player (it neither crits nor echoes).
            damage(
                stage,
                s,
                PlayerIdx::SOLO,
                aided(stage, s, b),
                Who::Companion(i),
                events,
            );
            return true;
        }
    }
    false
}
