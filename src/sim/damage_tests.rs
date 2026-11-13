//! Pins for where a player's damage meets a foe's hit points: the numbers, the events and the
//! credit of a hit, a damage over time tick, a kill burst and an echo, which all land through
//! `damage::land` (ADR-SIM-7); and a scan that keeps the write in that one place.

use super::entity::Who;
use super::state::{PlayerIdx, State};
use super::{Shot, World};
use crate::bench_fixtures::{time_limit_case, worst_case};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::id::EntityId;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A party's world and state with the tick's other work taken away.
fn bare(n: u8) -> (World, State) {
    let e: Engine = worst_case::warmed_up().into_party(n);
    let (w, s) = match e.run() {
        Run::V2 { world, state, .. } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let mut s = s.clone();
    s.runs.clear();
    s.orbs.clear();
    s.zones.clear();
    s.echoes.clear();
    (World::clone(w), s)
}

fn shot(s: &State, owner: EntityId, at: usize, damage: u32) -> Shot {
    Shot {
        at: s.enemies[at].at,
        owner,
        friendly: true,
        damage,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
    }
}

fn hit(w: &World, s: &mut State, me: u8, at: usize, damage: u32) -> Vec<DomainEvent> {
    let mut ev = Vec::new();
    let b = shot(s, slot(me).id(), at, damage);
    super::damage(w, s, slot(me), b, Who::Enemy(at), &mut ev);
    ev
}

/// A `Hit`'s damage, crit, echo, player and buffer.
type Seen = (u32, bool, bool, Option<EntityId>, Option<EntityId>);

fn hits(ev: &[DomainEvent]) -> Vec<Seen> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::Hit {
                damage,
                crit,
                echo,
                player,
                buffed_by,
                ..
            } => Some((*damage, *crit, *echo, *player, *buffed_by)),
            _ => None,
        })
        .collect()
}

fn deaths(ev: &[DomainEvent]) -> Vec<Option<EntityId>> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::Died { killer, .. } => Some(*killer),
            _ => None,
        })
        .collect()
}

#[test]
fn a_hit_lowers_hit_points_reports_and_raises_aggro() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    let aggro = s.aggro(Who::Player(slot(0)));
    let ev = hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 900);
    assert_eq!(
        hits(&ev),
        vec![(100, false, false, Some(slot(0).id()), None)]
    );
    assert!(deaths(&ev).is_empty());
    let gained = 100 * w.tuning.aggro_per_damage;
    assert_eq!(s.aggro(Who::Player(slot(0))), aggro + gained);
}

#[test]
fn a_killing_hit_reports_its_full_damage_and_credits_the_hitter() {
    let (w, mut s) = bare(3);
    s.players[slot(0)].echo_hit_pct = 50;
    s.enemies[0].hp = 900;
    s.enemies[0].defence = 0;
    let ev = hit(&w, &mut s, 1, 0, 1000);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(hits(&ev)[0].0, 1000, "not clamped to the hit points left");
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
    assert!(s.echoes.is_empty(), "a kill queues no echo");
}

#[test]
fn a_hit_that_leaves_the_foe_standing_queues_one_echo() {
    let (w, mut s) = bare(1);
    s.players[slot(0)].echo_hit_pct = 50;
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.echoes.len(), 1);
    assert_eq!(s.echoes[0].damage, 50);
}

#[test]
fn defence_reduces_the_damage_that_lands() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = u32::try_from(w.tuning.defence_k).expect("a small k");
    let ev = hit(&w, &mut s, 0, 0, 100);
    assert_eq!(hits(&ev)[0].0, 50);
    assert_eq!(s.enemies[0].hp, 950);
}

#[test]
fn a_hit_on_the_boss_lowers_its_hit_points() {
    let e: Engine = time_limit_case::warmed_up();
    let (w, s) = match e.run() {
        Run::V2 { world, state, .. } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let mut s = s.clone();
    let b = s.boss.as_mut().expect("the trial has a boss");
    b.hp = 10_000;
    b.defence = 0;
    b.invulnerable = 0;
    let mut ev = Vec::new();
    let sh = shot(&s, slot(0).id(), 0, 100);
    super::damage(w, &mut s, slot(0), sh, Who::Boss, &mut ev);
    assert_eq!(s.boss.as_ref().map(|b| b.hp), Some(9_900));
    assert_eq!(hits(&ev)[0].0, 100);
}

#[test]
fn an_echo_lands_as_an_echo_without_a_buffer_and_credits_its_kill() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 7;
    let id = s.enemies[0].id;
    let aggro = s.aggro(Who::Player(slot(1)));
    s.echoes.push(super::state::Echo {
        source: slot(1).id(),
        target: id,
        damage: 7,
        damage_type: None,
    });
    let mut ev = Vec::new();
    super::combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(hits(&ev), vec![(7, false, true, Some(slot(1).id()), None)]);
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
    assert!(s.echoes.is_empty());
    assert!(s.aggro(Who::Player(slot(1))) >= aggro);
}

#[test]
fn an_echo_on_a_standing_target_lowers_it_and_one_at_zero_is_skipped() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 100;
    s.enemies[1].hp = 0;
    let (a, b) = (s.enemies[0].id, s.enemies[1].id);
    for (target, damage) in [(a, 7), (b, 9)] {
        s.echoes.push(super::state::Echo {
            source: slot(0).id(),
            target,
            damage,
            damage_type: None,
        });
    }
    let mut ev = Vec::new();
    super::combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(s.enemies[0].hp, 93);
    assert_eq!(hits(&ev).len(), 1);
    assert!(s.echoes.is_empty());
}

/// A poison stack on enemy 0 that is due on the next tick: 3 stacks of 10.
fn poisoned(s: &mut State, by: u8) {
    use super::state::{DotState, StatusKind, StatusState};
    while !s.enemies[0].statuses.is_empty() {
        s.enemies[0].statuses.pop();
    }
    s.enemies[0].statuses.push(StatusState {
        kind: StatusKind::Poison,
        source: slot(by).id(),
        remaining: 10,
        total: 10,
        stacks: 3,
        value_pct: 0,
        dot: Some(DotState {
            damage: 10,
            damage_type: None,
            atk_up_pct: 0,
            type_bonus_pct: 0,
            interval: 1,
            next_in: 1,
        }),
    });
}

#[test]
fn a_damage_over_time_tick_lands_as_a_runed_hit_of_its_source() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    poisoned(&mut s, 2);
    let mut ev = Vec::new();
    super::entity::tick_dots(&w, &mut s, &mut ev);
    let ev = only(ev, s.enemies[0].id);
    assert_eq!(s.enemies[0].hp, 970);
    assert_eq!(
        hits(&ev),
        vec![(30, false, false, Some(slot(2).id()), None)]
    );
    assert!(ev.iter().any(|e| matches!(
        e,
        DomainEvent::Hit {
            rune: Some(crate::rune::Rune::Poison),
            ..
        }
    )));
}

#[test]
fn a_damage_over_time_kill_is_credited_to_the_stack_owner() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 20;
    s.enemies[0].defence = 0;
    poisoned(&mut s, 1);
    let mut ev = Vec::new();
    super::entity::tick_dots(&w, &mut s, &mut ev);
    let ev = only(ev, s.enemies[0].id);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
}

#[test]
fn a_hit_by_a_companion_or_an_unknown_source_is_slot_zeros() {
    let (w, mut s) = bare(2);
    s.enemies[0].hp = 1000;
    let mut ev = Vec::new();
    let b = shot(&s, EntityId(999), 0, 5);
    super::damage(&w, &mut s, slot(0), b, Who::Enemy(0), &mut ev);
    assert_eq!(hits(&ev)[0].3, Some(slot(0).id()));
}

#[test]
fn a_kill_burst_lands_a_plain_hit_without_an_echo() {
    let (w, mut s) = bare(1);
    s.players[slot(0)].burst_pct = 5;
    s.players[slot(0)].burst_radius = 100_000;
    s.players[slot(0)].echo_hit_pct = 50;
    s.enemies[1].at = s.enemies[0].at;
    s.enemies[0].hp = 0;
    s.enemies[0].max_hp = 1000;
    s.enemies[0].defence = 0;
    s.enemies[1].hp = 500;
    s.enemies[1].defence = 0;
    let mut ev = Vec::new();
    let mut n = 0;
    super::combat::bursts(&w, &mut s, slot(0), &mut n, &mut ev);
    let landed = hits(&only(ev, s.enemies[1].id));
    assert!(!landed.is_empty(), "the burst hurt a neighbour");
    assert!(landed.iter().all(|h| !h.1 && !h.2));
    assert_eq!(landed[0].0, 50);
    assert!(s.echoes.is_empty());
}

// --- the party factor (ADR-SIM-8) ------------------------------------------------------------

/// A party of `n` with the table `[100, 50, 25]` and a plain enemy 0 of `hp` hit points.
fn scaled(n: u8, hp: u32) -> (World, State) {
    let (mut w, mut s) = bare(n);
    w.tuning.party_damage_pct = [100, 50, 25];
    s.enemies[0].hp = hp;
    s.enemies[0].defence = 0;
    (w, s)
}

#[test]
fn the_bundled_table_changes_no_hit() {
    for n in 1..=3 {
        let (w, mut s) = bare(n);
        s.enemies[0].hp = 1000;
        s.enemies[0].defence = 0;
        hit(&w, &mut s, 0, 0, 100);
        assert_eq!(s.enemies[0].hp, 900, "{n} players");
    }
}

#[test]
fn one_hit_applies_the_factor_of_the_party_and_reports_the_raw_damage() {
    for (n, left) in [(1, 900), (2, 950), (3, 975)] {
        let (w, mut s) = scaled(n, 1000);
        let aggro = s.aggro(Who::Player(slot(0)));
        let ev = hit(&w, &mut s, 0, 0, 100);
        assert_eq!(s.enemies[0].hp, left, "{n} players");
        assert_eq!(hits(&ev)[0].0, 100);
        let gained = 100 * w.tuning.aggro_per_damage;
        assert_eq!(s.aggro(Who::Player(slot(0))), aggro + gained);
    }
}

#[test]
fn a_downed_or_out_player_still_counts_for_the_party_factor() {
    // Two present players: a hit of 100 loses 50 whether slot 1 is up, downed or out (ADR-SIM-8
    // point 1).
    for (hp, out) in [(100, false), (0, false), (0, true)] {
        let (w, mut s) = scaled(2, 1000);
        s.players[slot(1)].hp = hp;
        s.players[slot(1)].out = out;
        hit(&w, &mut s, 0, 0, 100);
        assert_eq!(s.enemies[0].hp, 950, "slot 1 hp {hp} out {out}");
    }
}

#[test]
fn the_factor_truncates_once_and_a_hit_applies_at_least_one() {
    for (n, shown, lowers) in [(3, 7, 1), (3, 99, 24), (3, 3, 1), (3, 1, 1), (2, 3, 1)] {
        let (w, mut s) = scaled(n, 1000);
        hit(&w, &mut s, 0, 0, shown);
        assert_eq!(1000 - s.enemies[0].hp, lowers, "{shown} at {n}");
    }
    let (w, mut s) = scaled(3, 1000);
    let mut ev = Vec::new();
    let how = super::damage::Landing {
        id: s.enemies[0].id,
        source: slot(0).id(),
        damage_type: None,
        match_up: crate::attack::MatchUp::None,
        rune: None,
        crit: false,
        echo: false,
        repeat: false,
        buffed_by: None,
    };
    super::damage::land(&w, &mut s, slot(0), Who::Enemy(0), 0, how, &mut ev);
    assert_eq!(s.enemies[0].hp, 1000, "nothing floors 0");
}

#[test]
fn the_floor_kills_and_a_short_hit_does_not() {
    let (w, mut s) = scaled(3, 1);
    s.players[slot(0)].echo_hit_pct = 50;
    let ev = hit(&w, &mut s, 1, 0, 1);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    let (w, mut s) = scaled(3, 30);
    s.players[slot(0)].echo_hit_pct = 50;
    let ev = hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 5);
    assert!(deaths(&ev).is_empty());
    assert_eq!(s.echoes.len(), 1);
    assert_eq!(s.echoes[0].damage, 50, "the echo reads the shown damage");
}

#[test]
fn the_boss_is_scaled_like_an_enemy() {
    let e: Engine = time_limit_case::warmed_up().into_party(3);
    let (w, s) = match e.run() {
        Run::V2 { world, state, .. } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let (mut w, mut s) = (World::clone(w), s.clone());
    w.tuning.party_damage_pct = [100, 50, 25];
    let b = s.boss.as_mut().expect("the trial has a boss");
    b.hp = 10_000;
    b.defence = 0;
    b.invulnerable = 0;
    let mut ev = Vec::new();
    let sh = shot(&s, slot(0).id(), 0, 100);
    super::damage(&w, &mut s, slot(0), sh, Who::Boss, &mut ev);
    assert_eq!(s.boss.as_ref().map(|b| b.hp), Some(9_975));
    assert_eq!(hits(&ev)[0].0, 100);
}

#[test]
fn an_ally_is_never_scaled() {
    let (w, mut s) = scaled(3, 1000);
    s.companions.push(s.enemies[0].clone());
    s.companions[0].hp = 5000;
    let mut ev = Vec::new();
    let how = super::damage::Landing {
        id: s.companions[0].id,
        source: s.enemies[0].id,
        damage_type: None,
        match_up: crate::attack::MatchUp::None,
        rune: None,
        crit: false,
        echo: false,
        repeat: false,
        buffed_by: None,
    };
    super::damage::land(&w, &mut s, slot(0), Who::Companion(0), 100, how, &mut ev);
    assert_eq!(s.companions[0].hp, 4900);
    assert_eq!(hits(&ev)[0].3, None);
}

#[test]
fn the_next_hit_reads_the_party_after_a_leave_or_a_join() {
    let (w, mut s) = scaled(3, 1000);
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 975);
    let before: Vec<u32> = s.enemies.iter().map(|e| e.hp).collect();
    s.players.leave(slot(2));
    assert_eq!(s.players.len(), 2);
    assert_eq!(s.enemies.iter().map(|e| e.hp).collect::<Vec<_>>(), before);
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 925);
    s.players.leave(slot(1));
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 825);
    let template = s.players.template();
    s.players.join(slot(1), &template, 1);
    s.players.join(slot(2), &template, 2);
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 800);
}

#[test]
fn a_damage_over_time_tick_is_scaled_once_on_its_shown_damage() {
    let (w, mut s) = scaled(3, 1000);
    poisoned(&mut s, 2);
    let mut ev = Vec::new();
    super::entity::tick_dots(&w, &mut s, &mut ev);
    let ev = only(ev, s.enemies[0].id);
    let shown = hits(&ev)[0].0;
    assert_eq!(shown, 30, "the shown damage is the unscaled tick");
    assert_eq!(1000 - s.enemies[0].hp, (shown * 25 / 100).max(1));
}

#[test]
fn an_echo_is_scaled_on_its_own_shown_damage() {
    let (w, mut s) = scaled(2, 1000);
    s.players[slot(0)].echo_hit_pct = 50;
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 950);
    assert_eq!(s.echoes[0].damage, 50);
    let mut ev = Vec::new();
    super::combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(hits(&ev)[0].0, 50);
    assert_eq!(s.enemies[0].hp, 925);
}

#[test]
fn a_kill_burst_shows_its_amount_and_applies_the_factor() {
    let (w, mut s) = scaled(3, 0);
    s.players[slot(0)].burst_pct = 5;
    s.players[slot(0)].burst_radius = 100_000;
    s.enemies[1].at = s.enemies[0].at;
    s.enemies[0].max_hp = 1000;
    s.enemies[1].hp = 500;
    s.enemies[1].defence = 0;
    let mut ev = Vec::new();
    let mut n = 0;
    super::combat::bursts(&w, &mut s, slot(0), &mut n, &mut ev);
    let landed = hits(&only(ev, s.enemies[1].id));
    assert_eq!(landed[0].0, 50);
    assert_eq!(s.enemies[1].hp, 488);
}

#[test]
fn the_identity_percent_returns_every_damage() {
    for shown in [0, 1, 2, 99, 100, 101, u32::MAX] {
        assert_eq!(super::damage::party_scaled(100, shown), shown);
    }
}

/// The events that name `id` as their target.
fn only(ev: Vec<DomainEvent>, id: EntityId) -> Vec<DomainEvent> {
    ev.into_iter()
        .filter(|e| match e {
            DomainEvent::Hit { target, .. } | DomainEvent::Died { entity: target, .. } => {
                *target == id
            }
            _ => false,
        })
        .collect()
}

#[allow(dead_code, reason = "shared with tests/, which uses more of it")]
mod data_root {
    include!("../../tests/data_root.rs");
}

/// The non-test part of a source file: everything before the first `#[cfg(test)]` that opens an
/// inline test module (`mod tests {`); a `#[cfg(test)] mod x_tests;` line is only a declaration.
fn code_of(text: &str) -> &str {
    let mut at = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]"
            && lines.peek().is_some_and(|n| n.trim_end().ends_with('{'))
        {
            return &text[..at];
        }
        at += line.len();
    }
    text
}

/// `file:line` of each `hp_mut(` call in `text`, a definition (`fn hp_mut`) not counted.
fn writes_in(file: &str, text: &str) -> Vec<String> {
    code_of(text)
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("hp_mut(") && !l.contains("fn hp_mut"))
        .map(|(i, _)| format!("{file}:{}", i + 1))
        .collect()
}

#[test]
fn the_guard_finds_a_write() {
    assert_eq!(writes_in("x.rs", "let hp = s.hp_mut(target);").len(), 1);
    assert!(writes_in("x.rs", "let hp = s.hp(target);").is_empty());
    assert!(
        writes_in(
            "x.rs",
            "fn hp_mut(&mut self)\n#[cfg(test)]\nmod t {\ns.hp_mut(t)"
        )
        .is_empty()
    );
}

#[test]
fn foe_hit_points_fall_in_one_place() {
    let dir = data_root::data_root().join("src/sim");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src/sim") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        // `damage.rs` is the write; `act.rs` is the heal, which raises hit points.
        if name == "damage.rs" || name == "act.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source file");
        found.extend(writes_in(name, &text));
    }
    assert!(
        found.is_empty(),
        "a hit point write outside `damage::land`: {found:?}"
    );
}

#[test]
fn the_rating_reads_the_rules_formula_and_keeps_no_copy() {
    let dir = data_root::data_root().join("src/sim/offer_rating.rs");
    let text = std::fs::read_to_string(dir).expect("offer_rating.rs");
    let code = code_of(&text);
    assert!(code.contains("attacks::swing_damage("));
    assert!(code.contains("combat::crit_damage("));
    for copy in ["fn scale_damage", "fn swing_damage", "fn crit_damage"] {
        assert!(!code.contains(copy), "{copy}");
    }
}
