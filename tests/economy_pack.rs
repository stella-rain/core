//! The economy and opening numbers are content (CS1b-2): the Rune Shards a clear pays, which
//! difficulty opens after which, the Lumine cost per rarity and the free weapon and perk set.
//! The pack holds the numbers the app and `run_setup` held as constants before the move.

use serde_json::{Value, json};
use stella_rain_core::pack::Pack;

fn pack() -> Value {
    serde_json::from_slice(Pack::BUNDLED).unwrap()
}

fn difficulties(pack: &Value) -> &Vec<Value> {
    pack["difficulties"]["difficulties"]
        .as_array()
        .expect("the pack has a difficulties list")
}

#[test]
fn the_pack_pays_today_s_rune_shards() {
    // The Rune Shards the app paid per difficulty before the numbers moved into the pack.
    let pack = pack();
    let got: Vec<(String, Option<u64>)> = difficulties(&pack)
        .iter()
        .map(|d| {
            (
                d["id"].as_str().unwrap().to_owned(),
                d["rune_shards"].as_u64(),
            )
        })
        .collect();
    let want: Vec<(String, Option<u64>)> =
        [("easy", 10), ("normal", 15), ("hard", 20), ("rainy", 30)]
            .iter()
            .map(|(id, n)| ((*id).to_owned(), Some(*n)))
            .collect();
    assert_eq!(got, want);
}

/// The rule the app applied before the numbers moved into the pack: the first two difficulties of
/// the file are open; every later one opens when the one before it in the file is cleared.
fn app_open(order: &[&str], id: &str, cleared: &[&str]) -> bool {
    let Some(i) = order.iter().position(|d| *d == id) else {
        return false;
    };
    i < 2 || cleared.contains(&order[i - 1])
}

#[test]
fn the_pack_opens_difficulties_as_the_app_does_today() {
    let pack = pack();
    let list = difficulties(&pack);
    let order: Vec<&str> = list.iter().map(|d| d["id"].as_str().unwrap()).collect();
    let after: Vec<Value> = list
        .iter()
        .map(|d| d.get("opens_after").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        after,
        vec![Value::Null, Value::Null, json!("normal"), json!("hard")]
    );
    for mask in 0..16u32 {
        let cleared: Vec<&str> = order
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, id)| *id)
            .collect();
        for (d, id) in list.iter().zip(&order) {
            let by_pack = match d.get("opens_after").and_then(Value::as_str) {
                None => true,
                Some(a) => cleared.contains(&a),
            };
            assert_eq!(
                by_pack,
                app_open(&order, id, &cleared),
                "{id} with {cleared:?}"
            );
        }
    }
}

#[test]
fn the_pack_prices_rarities_as_run_setup_did() {
    // The costs `run_setup::lumine_cost` returned before they moved into the rewards file.
    let pack = pack();
    assert_eq!(
        pack["rewards"]["rarities"],
        json!([
            {"rarity": "common", "lumine_cost": 1},
            {"rarity": "uncommon", "lumine_cost": 2},
            {"rarity": "rare", "lumine_cost": 3},
            {"rarity": "epic", "lumine_cost": 4},
            {"rarity": "legendary", "lumine_cost": 6},
            {"rarity": "supernova", "lumine_cost": 10},
        ])
    );
}

#[test]
fn the_pack_names_today_s_free_picks() {
    // The free weapon and perk set the app's preset screen named before they moved into the pack.
    let pack = pack();
    let su = &pack["starting_unlocks"];
    assert_eq!(su["free_weapon"], "giant_sword");
    assert_eq!(su["free_perk_set"], "duelist");
    assert_eq!(su["weapons"][0], su["free_weapon"]);
}
