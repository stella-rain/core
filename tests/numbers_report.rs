//! The bot numbers report: one `RUN` line per bot run (key=value) over fixed seeds, so a later numbers pass
//! reruns the same measurements with one command. It measures drop counts and category share, Starlight per
//! run, slot pace by tick and Stardust per run, and changes nothing: no corpus, hash, golden or pin reads it.
//! Knobs (environment): MODE DIFF SEED0 SEEDS TICKS BOT DWA DWD DWM (drop weights, percent) LABEL; the bot seed is
//! the world seed unless BOT is set. Run by hand:
//! `MODE=time_limit DIFF=easy SEEDS=30 TICKS=18000 cargo test --release --test integration numbers_report:: -- --ignored --nocapture --test-threads=1`.
use crate::party_bot::RigParty;
use crate::run_play::party_inputs_with;
use std::collections::BTreeMap;
use stella_rain_core::engine::Outcome;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::EquipAction;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::snapshot::HeldView;

#[expect(clippy::disallowed_methods, reason = "test knob")]
fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[test]
#[ignore = "a probe, run by hand"]
fn numbers_probe() {
    let mode = env("MODE", "time_limit");
    let diff = env("DIFF", "easy");
    let seed0: u64 = env("SEED0", "1").parse().unwrap();
    let seeds: u64 = env("SEEDS", "6").parse().unwrap();
    let ticks: u32 = env("TICKS", "18000").parse().unwrap();
    let bot_fixed: Option<u64> = Some(env("BOT", "0").parse().unwrap()).filter(|b| *b > 0);
    let dw = [("attack", "DWA"), ("defence", "DWD"), ("mobility", "DWM")];
    let rewards = Rewards::bundled();
    let cat_of = |id: &str| -> String {
        rewards
            .rewards()
            .iter()
            .find(|r| r.id.as_str() == id)
            .and_then(|r| r.category.as_ref().map(|c| c.as_str().to_string()))
            .unwrap_or_else(|| "none".into())
    };
    for seed in seed0..seed0 + seeds {
        let bot = bot_fixed.unwrap_or(seed);
        let mut case = RigParty::new(1)
            .mode(&mode)
            .difficulty(&diff)
            .seed(seed)
            .bot_seed(bot)
            .equipment()
            .builds()
            .ticks(ticks)
            .case("probe");
        for (cat, key) in dw {
            let v: u32 = env(key, "0").parse().unwrap();
            if v > 0 {
                case.setup.drop_weights.insert(cat.into(), v);
            }
        }
        let mut rec = RunRecorder::start_party(&case.setup, 1).unwrap();
        let mut prev_inv: Vec<HeldView> = rec.engine().snapshot().players[0].inventory.clone();
        let mut prev_star = 0u32;
        let (mut gross, mut extracts, mut crafts, mut equips, mut replaces, mut treepicks) =
            (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
        let mut first_craft: Option<u32> = None;
        let mut ids: BTreeMap<String, u32> = BTreeMap::new();
        let mut ex_rune = 0u32;
        let mut ex_by: BTreeMap<String, u32> = BTreeMap::new();
        let mut first_rune: Option<u32> = None;
        let mut taken: BTreeMap<String, u32> = BTreeMap::new();
        let mut cats: BTreeMap<String, u32> = BTreeMap::new();
        let mut runes_in = 0u32;
        let mut stars_in = 0u32;
        let mut lvl_first: BTreeMap<u8, u32> = BTreeMap::new();
        let mut max_lvl_3000 = 0u8;
        let mut stardust = 0u32;
        let mut charlvl = 0u32;
        let mut t = 0u32;
        while t < ticks {
            if rec.engine().outcome() != Outcome::Running {
                break;
            }
            let inputs = party_inputs_with(&rec, t, case.bot.as_ref());
            let act = inputs[0].action;
            rec.step(&inputs).unwrap();
            t += 1;
            match act {
                Some(EquipAction::Extract { item }) => {
                    extracts += 1;
                    match prev_inv.get(item as usize) {
                        Some(HeldView::Rune { .. }) => ex_rune += 1,
                        Some(HeldView::Star { rarity, .. }) => {
                            *ex_by.entry(format!("{rarity:?}")).or_default() += 1
                        }
                        None => {}
                    }
                }
                Some(EquipAction::Craft { .. }) => {
                    crafts += 1;
                    first_craft.get_or_insert(t);
                }
                Some(EquipAction::Equip { .. }) => equips += 1,
                Some(EquipAction::Replace { .. }) => replaces += 1,
                Some(EquipAction::TreePick { .. }) => treepicks += 1,
                None => {}
            }
            let took = rec
                .engine()
                .events()
                .iter()
                .filter(|e| matches!(e, DomainEvent::StarTaken { .. }))
                .count();
            for e in rec.engine().events() {
                if let DomainEvent::StarTaken { rarity, .. } = e {
                    *taken.entry(format!("{rarity:?}")).or_default() += 1;
                }
            }
            let snap = rec.engine().snapshot();
            let p = &snap.players[0];
            if took > 0 && act.is_none() && p.inventory.len() > prev_inv.len() {
                for item in &p.inventory[prev_inv.len()..] {
                    match item {
                        HeldView::Star { id, .. } => {
                            stars_in += 1;
                            *ids.entry(id.as_str().to_string()).or_default() += 1;
                            *cats.entry(cat_of(id.as_str())).or_default() += 1;
                        }
                        HeldView::Rune { .. } => {
                            runes_in += 1;
                            first_rune.get_or_insert(t);
                        }
                    }
                }
            }
            prev_inv = p.inventory.clone();
            if p.starlight > prev_star {
                gross += p.starlight - prev_star;
            }
            prev_star = p.starlight;
            let maxl = p.slots.iter().map(|s| s.level).max().unwrap_or(0);
            for n in 2..=10u8 {
                if maxl >= n {
                    lvl_first.entry(n).or_insert(t);
                }
            }
            if t == 3000 {
                max_lvl_3000 = maxl;
            }
            stardust = snap.stardust.unwrap_or(0);
            charlvl = snap.progress.as_ref().map(|g| g.level).unwrap_or(0);
        }
        let out = rec.engine().outcome();
        println!(
            "RUN label={} mode={mode} diff={diff} seed={seed} bot={bot} end={t} outcome={out:?} \
             stardust10={stardust} charlvl={charlvl} starlight_gross={gross} \
             extracts={extracts} crafts={crafts} first_craft={first_craft:?} equips={equips} \
             replaces={replaces} treepicks={treepicks} stars_in={stars_in} runes_in={runes_in} \
             first_rune={first_rune:?} ids={ids:?} ex_rune={ex_rune} ex_by={ex_by:?} taken={taken:?} cats={cats:?} maxslot3000={max_lvl_3000} \
             slotlvl_first={lvl_first:?}",
            env("LABEL", "base"),
        );
    }
}

#[test]
#[ignore = "a probe, run by hand"]
fn numbers_probe_weights() {
    use std::collections::BTreeMap;
    let rw = stella_rain_core::rewards::Rewards::bundled();
    let mut m: BTreeMap<String, u32> = BTreeMap::new();
    for r in rw.rewards() {
        if r.deprecated || r.category.is_none() {
            continue;
        }
        *m.entry(format!(
            "{:?}/{}",
            r.rarity,
            r.category.as_ref().unwrap().as_str()
        ))
        .or_default() += u32::from(r.weight);
    }
    println!("WEIGHTS {m:?}");
    println!("POOL {:?}", rw.default_pool(2).len());
}
