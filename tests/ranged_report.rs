//! The ranged-attack report: what the enemies' ranged attacks do on bot equipment runs, so a later
//! combat pass reruns the same measurements with one command. Per run and party size it prints a
//! `RUN` line (ranged starts per pool, tokens held and cooling, hits and damage per player per 60
//! and 120 ticks, lanes seen) and `KIND`, `OFFVIEW` and `WINDOW` lines. It reads only the public
//! snapshot, events and content, and changes nothing: no corpus, hash, golden or pin reads it.
//! Knobs (environment): MODE DIFF SEED0 SEEDS TICKS BOT PARTIES ZONE_AT LINE_AT LABEL. The bundled
//! mode brings its first ranged wave at tick 10800 and the bots' runs end near tick 3000, so the
//! probe moves the two ranged waves to ZONE_AT and LINE_AT (default 600) in memory; ZONE_AT=10800
//! LINE_AT=25200 keeps the shipped timeline. Run by hand:
//! `SEEDS=6 cargo test --release --test integration ranged_report:: -- --ignored --nocapture --test-threads=1`.
use crate::party_bot::RigParty;
use crate::run_play::party_inputs_with;
use std::collections::{BTreeMap, BTreeSet};
use stella_rain_core::attack::{AttackPhase, AttackRef, Emitter, PhaseKind, ZoneShape};
use stella_rain_core::behaviour::{Action, Agent};
use stella_rain_core::engine::Outcome;
use stella_rain_core::event::{DomainEvent, HitKind};
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::limits::{VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::version::Versions;

#[expect(clippy::disallowed_methods, reason = "test knob")]
fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

/// What an enemy is, for who the tokens and the view bar apply to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Regular,
    Elite,
    Companion,
    Boss,
}

fn class_of(kind: EntityKind, elite: bool) -> Class {
    match kind {
        EntityKind::Companion => Class::Companion,
        EntityKind::Boss | EntityKind::BossPart => Class::Boss,
        EntityKind::Shrine => Class::Elite,
        EntityKind::Enemy if elite => Class::Elite,
        EntityKind::Enemy => Class::Regular,
    }
}

/// Whether an attack lays a line zone (the line pool, `RangedPool::of`).
fn lays_line(list: &[Emitter]) -> bool {
    list.iter().any(|n| match n {
        Emitter::Zone { shape, .. } => matches!(shape, ZoneShape::Line { .. }),
        Emitter::Orb { burst, .. } => lays_line(burst),
        Emitter::Repeat { body, .. }
        | Emitter::Ring { body, .. }
        | Emitter::Spread { body, .. }
        | Emitter::Rotate { body, .. } => lays_line(body),
        Emitter::Wait { .. } => false,
    })
}

/// A ranged attack's identity as the snapshot shows it: the sprite and the ticks of its first
/// phase (the wind-up a `Telegraph` announces); `true` is the line pool.
type Signature = (String, u32);

/// Every ranged rule of the mode's agents (waves, summons), by signature; a signature that a
/// non-ranged rule of the same sprite shares is also returned, as ambiguous.
fn ranged_table(content: &RunContent) -> (BTreeMap<Signature, bool>, BTreeSet<Signature>) {
    let mut ranged = BTreeMap::new();
    let mut plain = BTreeSet::new();
    fn walk(
        content: &RunContent,
        a: &Agent,
        ranged: &mut BTreeMap<Signature, bool>,
        plain: &mut BTreeSet<Signature>,
    ) {
        for rule in &a.rules {
            match &rule.action {
                Action::Attack { attack, ranged: r } => {
                    let phases: &[AttackPhase] = match attack {
                        AttackRef::Inline(p) => p,
                        AttackRef::Preset { id, .. } => content
                            .attack_presets()
                            .definition(id.as_str())
                            .unwrap_or(&[]),
                    };
                    let Some(first) = phases.first() else {
                        continue;
                    };
                    let sig = (a.base.as_str().to_string(), first.ticks);
                    if *r {
                        ranged.insert(sig, phases.iter().any(|p| lays_line(&p.spawn)));
                    } else {
                        plain.insert(sig);
                    }
                }
                Action::Summon { agent, .. } => walk(content, agent, ranged, plain),
                _ => {}
            }
        }
    }
    let tl = content.time_limit();
    for w in tl.waves() {
        walk(content, &w.enemy, &mut ranged, &mut plain);
    }
    let ambiguous = ranged
        .keys()
        .filter(|s| plain.contains(*s))
        .cloned()
        .collect();
    (ranged, ambiguous)
}

/// The probe's content: the bundled files with the two ranged waves (the waves whose enemy has a
/// ranged rule, in file order: the zone one, then the line one) moved to the given ticks.
fn content_with(zone_at: u32, line_at: u32) -> RunContent {
    let base = RunContent::bundled();
    let mut v: serde_json::Value = serde_json::from_slice(base.time_limit().bytes()).unwrap();
    let mut moved = 0;
    for w in v["waves"].as_array_mut().unwrap() {
        let is_ranged = w["enemy"]["rules"]
            .as_array()
            .is_some_and(|r| r.iter().any(|x| x["do"]["ranged"] == true));
        if is_ranged {
            w["at_tick"] = (if moved == 0 { zone_at } else { line_at }).into();
            moved += 1;
        }
    }
    assert_eq!(moved, 2, "the bundled mode has a zone wave and a line wave");
    let tl = TimeLimit::parse(&serde_json::to_vec(&v).unwrap(), Versions::CURRENT).unwrap();
    base.clone().with_time_limit(tl)
}

/// The view's box around the party (`Area::of_view`): its centre and half extents, in `Fx` units.
struct View {
    cx: i64,
    cy: i64,
    hx: i64,
    hy: i64,
}

fn view_of(snap: &Snapshot) -> View {
    let (w, h) = (i64::from(snap.map.width.0), i64::from(snap.map.height.0));
    let axis = |t: i64, side: i64, half: i64| {
        if side <= 2 * half {
            side / 2
        } else {
            t.clamp(half, side - half)
        }
    };
    let (hw, hh) = (
        i64::from(Fx::px(VIEW_HALF_WIDTH).0),
        i64::from(Fx::px(VIEW_HALF_HEIGHT).0),
    );
    let up: Vec<_> = snap
        .players
        .iter()
        .filter(|p| p.down.is_none() && !p.out)
        .collect();
    let who: Vec<_> = if up.is_empty() {
        snap.players.iter().collect()
    } else {
        up
    };
    let cs: Vec<(i64, i64)> = who
        .iter()
        .map(|p| {
            (
                axis(i64::from(p.at.x.0), w, hw),
                axis(i64::from(p.at.y.0), h, hh),
            )
        })
        .collect();
    let lo = (
        cs.iter().map(|c| c.0).min().unwrap_or(0),
        cs.iter().map(|c| c.1).min().unwrap_or(0),
    );
    let hi = (
        cs.iter().map(|c| c.0).max().unwrap_or(0),
        cs.iter().map(|c| c.1).max().unwrap_or(0),
    );
    let mid = |lo: i64, hi: i64| lo + (hi - lo) / 2;
    let (cx, cy) = (mid(lo.0, hi.0), mid(lo.1, hi.1));
    View {
        cx,
        cy,
        hx: hi.0 - cx,
        hy: hi.1 - cy,
    }
}

impl View {
    fn holds(&self, p: Point) -> bool {
        (i64::from(p.x.0) - self.cx).abs() <= i64::from(Fx::px(VIEW_HALF_WIDTH).0) + self.hx
            && (i64::from(p.y.0) - self.cy).abs() <= i64::from(Fx::px(VIEW_HALF_HEIGHT).0) + self.hy
    }
}

/// The distance in whole pixels from `p` to the nearest player that is up.
fn nearest_px(snap: &Snapshot, p: Point) -> i64 {
    let best = snap
        .players
        .iter()
        .filter(|q| q.down.is_none() && !q.out)
        .map(|q| {
            let (dx, dy) = (i64::from(q.at.x.0 - p.x.0), i64::from(q.at.y.0 - p.y.0));
            dx * dx + dy * dy
        })
        .min()
        .unwrap_or(0);
    best.isqrt() / i64::from(stella_rain_core::fixed::UNITS_PER_PX)
}

#[derive(Default)]
struct Spread {
    n: u32,
    sum: i64,
    min: i64,
    max: i64,
}

impl Spread {
    fn add(&mut self, v: i64) {
        if self.n == 0 {
            (self.min, self.max) = (v, v);
        }
        self.n += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }
    fn show(&self) -> String {
        format!(
            "n={} min={} mean={} max={}",
            self.n,
            self.min,
            self.sum / i64::from(self.n.max(1)),
            self.max
        )
    }
}

/// The window tallies of one player: hits and damage per window of `len` ticks.
fn windows(len: u32, per_tick: &[(u32, u32)]) -> (u32, u32, u32, u32, u32) {
    let mut hits = Vec::new();
    let mut dmg = Vec::new();
    for chunk in per_tick.chunks(len as usize) {
        hits.push(chunk.iter().map(|c| c.0).sum::<u32>());
        dmg.push(chunk.iter().map(|c| c.1).sum::<u32>());
    }
    let n = u32::try_from(hits.len()).unwrap();
    (
        n,
        hits.iter().sum(),
        hits.iter().copied().max().unwrap_or(0),
        dmg.iter().sum(),
        dmg.iter().copied().max().unwrap_or(0),
    )
}

/// One run's knobs.
struct Knobs<'a> {
    mode: &'a str,
    diff: &'a str,
    seed: u64,
    bot: u64,
    n: u8,
    ticks: u32,
    label: &'a str,
}

fn probe_run(content: &RunContent, k: &Knobs) {
    let Knobs {
        mode,
        diff,
        seed,
        bot,
        n,
        ticks,
        label,
    } = *k;
    let case = RigParty::new(n)
        .mode(mode)
        .difficulty(diff)
        .seed(seed)
        .bot_seed(bot)
        .equipment()
        .builds()
        .ticks(ticks)
        .case("ranged");
    let mut rec = RunRecorder::start_party_with(&case.setup, content, n).unwrap();
    let rules = content.rules();
    let cooldown = rules.ranged_token_cooldown_ticks();
    let cap = [rules.ranged_zone_tokens(), rules.ranged_line_tokens()];
    let (table, ambiguous) = ranged_table(content);
    let players = usize::from(n);

    let mut held_since: BTreeMap<EntityId, (usize, i64)> = BTreeMap::new(); // id -> pool, nearest px so far
    let mut cooling: Vec<(u32, usize)> = Vec::new();
    let mut shooters: BTreeSet<EntityId> = BTreeSet::new();
    // The nearest player's distance, in px, of each shooter on each tick it stands (RA4).
    let mut spacing: Vec<i64> = Vec::new();
    let mut starts = [0u32; 2];
    let mut start_out_of_view = 0u32;
    let mut held_max = [0u32; 2];
    let mut held_sum = [0u64; 2];
    let mut cool_max = [0u32; 2];
    let mut cool_sum = [0u64; 2];
    let mut over_cap = 0u32;
    let mut starts_while_full = 0u32;
    let mut kinds: BTreeMap<(String, usize), (Spread, Spread, u32)> = BTreeMap::new();
    // class -> (telegraphs, telegraphs with the source outside the view)
    let mut tele: BTreeMap<Class, (u32, u32)> = BTreeMap::new();
    // (class, in view, hit kind) -> hits on a player
    let mut hurt: BTreeMap<(Class, bool, &'static str), u32> = BTreeMap::new();
    let mut zones_off: BTreeMap<&'static str, (u32, u32)> = BTreeMap::new(); // phase -> (zone-ticks, outside view)
    let mut lane_ticks = 0u32;
    let mut lane_max = 0usize;
    let mut lanes_seen: BTreeSet<(i32, i32, i32, i32, u32)> = BTreeSet::new();
    let mut lane_start_px = Spread::default();
    let mut lane_out_of_view = 0u32;
    let mut per_player: Vec<Vec<(u32, u32)>> = vec![Vec::new(); players];
    let mut ranged_hits_on = vec![0u32; players];
    let mut hit_mismatch = 0u32;
    let mut unattributed = 0u32;
    let mut prev_hp: Vec<u32> = rec
        .engine()
        .snapshot()
        .players
        .iter()
        .map(|p| p.hp)
        .collect();
    let mut prev_classes: BTreeMap<EntityId, (Class, String, Point)> = BTreeMap::new();
    let mut t = 0u32;
    while t < ticks && rec.engine().outcome() == Outcome::Running {
        let inputs = party_inputs_with(&rec, t, case.bot.as_ref());
        rec.step(&inputs).unwrap();
        t += 1;
        let snap = rec.engine().snapshot();
        let ev = rec.engine().events();
        let view = view_of(&snap);
        let mut here: BTreeMap<EntityId, (Class, String, Point)> = BTreeMap::new();
        let mut attack_of: BTreeMap<EntityId, (PhaseKind, u8, u32)> = BTreeMap::new();
        for e in &snap.entities {
            here.insert(
                e.id,
                (
                    class_of(e.kind, e.elite),
                    e.asset.as_str().to_string(),
                    e.at,
                ),
            );
            if let Some(a) = &e.attack {
                attack_of.insert(e.id, (a.kind, a.phase, a.ticks));
            }
        }
        // Starts: a telegraph of a regular enemy whose wind-up matches a ranged rule.
        for e in ev {
            let DomainEvent::Telegraph {
                source,
                ticks: wind,
            } = e
            else {
                continue;
            };
            let Some((class, asset, at)) = here.get(source).cloned() else {
                continue;
            };
            let slot = tele.entry(class).or_default();
            slot.0 += 1;
            if !view.holds(at) {
                slot.1 += 1;
            }
            if class != Class::Regular {
                continue;
            }
            let Some(&line) = table.get(&(asset.clone(), *wind)) else {
                continue;
            };
            let pool = usize::from(line);
            let d = nearest_px(&snap, at);
            starts[pool] += 1;
            if !view.holds(at) {
                start_out_of_view += 1;
            }
            let held_now = held_since.values().filter(|h| h.0 == pool).count();
            if u32::try_from(held_now).unwrap() >= cap[pool] {
                starts_while_full += 1;
            }
            held_since.insert(*source, (pool, d));
            shooters.insert(*source);
            let k = kinds.entry((asset, pool)).or_default();
            k.0.add(d);
            k.2 += 1;
        }
        for id in &shooters {
            if let Some((_, _, at)) = here.get(id) {
                spacing.push(nearest_px(&snap, *at));
            }
        }
        // Tokens: held while the run is in its wind-up or active phase; freed ones cool.
        let ids: Vec<EntityId> = held_since.keys().copied().collect();
        for id in ids {
            let (pool, near) = held_since[&id];
            match (here.get(&id), attack_of.get(&id)) {
                (Some((_, _, at)), Some((kind, _, _))) if *kind != PhaseKind::Recovery => {
                    let d = nearest_px(&snap, *at);
                    held_since.insert(id, (pool, near.min(d)));
                }
                (_, _) => {
                    let (_, near) = held_since.remove(&id).unwrap();
                    cooling.push((t + cooldown, pool));
                    if let Some(prev) = prev_classes.get(&id).or(here.get(&id)) {
                        if let Some(k) = kinds.get_mut(&(prev.1.clone(), pool)) {
                            k.1.add(near);
                        }
                    }
                }
            }
        }
        cooling.retain(|(until, _)| *until > t);
        for p in 0..2 {
            let h = u32::try_from(held_since.values().filter(|x| x.0 == p).count()).unwrap();
            let c = u32::try_from(cooling.iter().filter(|x| x.1 == p).count()).unwrap();
            held_max[p] = held_max[p].max(h);
            cool_max[p] = cool_max[p].max(c);
            held_sum[p] += u64::from(h);
            cool_sum[p] += u64::from(c);
            if h > cap[p] {
                over_cap += 1;
            }
        }
        // Hits on players: by owner class and where the owner stood.
        let hits: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                DomainEvent::PlayerHit { hp_left, source } => Some((*hp_left, source.clone())),
                _ => None,
            })
            .collect();
        let healed = |i: usize| -> u32 {
            ev.iter()
                .filter_map(|e| match e {
                    DomainEvent::Healed { target, amount, .. } if target.0 as usize == i => {
                        Some(*amount)
                    }
                    _ => None,
                })
                .sum()
        };
        let mut dropped = Vec::new();
        for i in 0..players {
            let hp = snap.players[i].hp;
            let lost = (prev_hp[i] + healed(i)).saturating_sub(hp);
            per_player[i].push((u32::from(lost > 0), lost));
            if lost > 0 {
                dropped.push(i);
            }
            prev_hp[i] = hp;
        }
        if hits.len() != dropped.len() {
            hit_mismatch += 1;
        }
        for (idx, (_, src)) in hits.iter().enumerate() {
            let owner = here.get(&src.owner).or(prev_classes.get(&src.owner));
            let (class, at) = match owner {
                Some((c, _, p)) => (*c, Some(*p)),
                None => (Class::Regular, None),
            };
            let in_view = at.is_none_or(|p| view.holds(p));
            let kind = match src.kind {
                HitKind::Attack => "attack",
                HitKind::Zone => "zone",
                HitKind::Link => "link",
            };
            *hurt.entry((class, in_view, kind)).or_default() += 1;
            if shooters.contains(&src.owner) {
                // The player is the one that dropped to `hp_left` in this tick, if only one did.
                let hp_left = hits[idx].0;
                let who: Vec<usize> = dropped
                    .iter()
                    .copied()
                    .filter(|i| snap.players[*i].hp == hp_left)
                    .collect();
                if let [one] = who[..] {
                    ranged_hits_on[one] += 1;
                } else {
                    unattributed += 1;
                }
            }
        }
        // Zones of enemies: how many are outside the view, and the lanes.
        let mut lines_now = 0usize;
        for z in snap.zones.iter().filter(|z| !z.friendly) {
            let phase = if z.warn_only {
                "warn"
            } else if z.phase == PhaseKind::WindUp {
                "wind_up"
            } else {
                "active"
            };
            let slot = zones_off.entry(phase).or_default();
            slot.0 += 1;
            if !view.holds(z.at) {
                slot.1 += 1;
            }
            if let ZoneShape::Line { .. } = z.shape {
                lines_now += 1;
                if z.phase == PhaseKind::WindUp && z.ticks_left == z.ticks && !z.warn_only {
                    let key = (z.at.x.0, z.at.y.0, z.direction.x, z.direction.y, t);
                    if lanes_seen.insert(key) {
                        lane_start_px.add(nearest_px(&snap, z.at));
                        if !view.holds(z.at) {
                            lane_out_of_view += 1;
                        }
                    }
                }
            }
        }
        lane_ticks += u32::from(lines_now > 0);
        lane_max = lane_max.max(lines_now);
        prev_classes = here;
    }
    let out = rec.engine().outcome();
    let mean = |s: u64| s * 100 / u64::from(t.max(1));
    println!(
        "RUN label={label} mode={mode} diff={diff} seed={seed} bot={bot} n={n} end={t} outcome={out:?} \
         starts_zone={} starts_line={} starts_out_of_view={start_out_of_view} starts_while_pool_full={starts_while_full} \
         held_max={held_max:?} held_mean_x100=[{}, {}] cooling_max={cool_max:?} cooling_mean_x100=[{}, {}] over_cap_ticks={over_cap} \
         lanes={} lane_ticks={lane_ticks} lanes_at_once_max={lane_max} lane_start_px=[{}] lanes_start_out_of_view={lane_out_of_view} \
         ambiguous_signatures={} hit_count_mismatch_ticks={hit_mismatch} ranged_hits_unattributed={unattributed}",
        starts[0],
        starts[1],
        mean(held_sum[0]),
        mean(held_sum[1]),
        mean(cool_sum[0]),
        mean(cool_sum[1]),
        lanes_seen.len(),
        lane_start_px.show(),
        ambiguous.len(),
    );
    spacing.sort_unstable();
    let share =
        |limit: i64| spacing.iter().filter(|d| **d < limit).count() * 1000 / spacing.len().max(1);
    let at = |pct: usize| {
        spacing
            .get(spacing.len().saturating_sub(1) * pct / 100)
            .copied()
    };
    println!(
        "SPACING label={label} seed={seed} n={n} shooter_ticks={} p10_px={:?} p50_px={:?} share_x1000_under_48px={} share_x1000_under_160px={}",
        spacing.len(),
        at(10),
        at(50),
        share(48),
        share(160),
    );
    for ((asset, pool), (start, held, count)) in &kinds {
        println!(
            "KIND label={label} seed={seed} n={n} asset={asset} pool={} starts={count} start_px=[{}] nearest_while_held_px=[{}]",
            if *pool == 1 { "line" } else { "zone" },
            start.show(),
            held.show(),
        );
    }
    for (class, (all, out)) in &tele {
        println!(
            "OFFVIEW label={label} seed={seed} n={n} telegraphs class={class:?} total={all} source_outside_view={out}"
        );
    }
    for ((class, in_view, kind), c) in &hurt {
        println!(
            "OFFVIEW label={label} seed={seed} n={n} hits_on_players class={class:?} owner_in_view={in_view} kind={kind} count={c}"
        );
    }
    for (phase, (all, out)) in &zones_off {
        println!(
            "OFFVIEW label={label} seed={seed} n={n} enemy_zone_ticks phase={phase} total={all} centre_outside_view={out}"
        );
    }
    for (i, ticks_of) in per_player.iter().enumerate() {
        for len in [60u32, 120] {
            let (wins, hits, hits_max, dmg, dmg_max) = windows(len, ticks_of);
            println!(
                "WINDOW label={label} seed={seed} n={n} player={i} len={len} windows={wins} hits={hits} hits_max={hits_max} \
                 hits_mean_x100={} damage={dmg} damage_max={dmg_max} damage_per_window={} ranged_hits={}",
                u64::from(hits) * 100 / u64::from(wins.max(1)),
                u64::from(dmg) / u64::from(wins.max(1)),
                ranged_hits_on[i],
            );
        }
    }
}

#[test]
#[ignore = "a probe, run by hand"]
fn ranged_probe() {
    let mode = env("MODE", "time_limit");
    let diff = env("DIFF", "easy");
    let seed0: u64 = env("SEED0", "1").parse().unwrap();
    let seeds: u64 = env("SEEDS", "6").parse().unwrap();
    let ticks: u32 = env("TICKS", "20000").parse().unwrap();
    let bot_fixed: Option<u64> = Some(env("BOT", "0").parse().unwrap()).filter(|b| *b > 0);
    let zone_at: u32 = env("ZONE_AT", "600").parse().unwrap();
    let line_at: u32 = env("LINE_AT", "600").parse().unwrap();
    let label = env("LABEL", "base");
    let content = content_with(zone_at, line_at);
    for part in env("PARTIES", "1,3").split(',') {
        let n: u8 = part.trim().parse().unwrap();
        for seed in seed0..seed0 + seeds {
            let knobs = Knobs {
                mode: &mode,
                diff: &diff,
                seed,
                bot: bot_fixed.unwrap_or(seed),
                n,
                ticks,
                label: &label,
            };
            probe_run(&content, &knobs);
        }
    }
}
