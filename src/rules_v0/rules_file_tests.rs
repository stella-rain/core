//! `content/rules.json` holds today's designer-facing rules constants (CS1b-3). Temporary: it
//! lives until the constants are deleted from Rust, and the replay corpus holds the values after.

use serde_json::Value;

use super::*;

#[test]
fn the_rules_file_holds_today_s_constants() {
    let file: Value = serde_json::from_str(include_str!("../../content/rules.json")).unwrap();
    let want: [(&str, i64); 24] = [
        ("crowd_target", 200),
        ("cap_headroom_pct", 30),
        ("player_speed", i64::from(PLAYER_SPEED)),
        ("player_radius", i64::from(PLAYER_RADIUS)),
        ("orb_radius", i64::from(ORB_RADIUS)),
        ("hit_invulnerable_ticks", i64::from(HIT_INVULNERABLE_TICKS)),
        ("aggro_per_damage", i64::from(AGGRO_PER_DAMAGE)),
        ("aggro_per_buff", i64::from(AGGRO_PER_BUFF)),
        ("aggro_per_heal_point", i64::from(AGGRO_PER_HEAL_POINT)),
        ("aggro_decay", i64::from(AGGRO_DECAY)),
        ("leash_reach_x", i64::from(LEASH_REACH_X.0)),
        ("leash_reach_y", i64::from(LEASH_REACH_Y.0)),
        ("leash_land_x", i64::from(LEASH_LAND_X.0)),
        ("leash_land_y", i64::from(LEASH_LAND_Y.0)),
        ("spawn_margin", i64::from(SPAWN_MARGIN.0)),
        (
            "ranged_token_cooldown_ticks",
            i64::from(RANGED_TOKEN_COOLDOWN_TICKS),
        ),
        ("ranged_zone_tokens", i64::from(RANGED_ZONE_TOKENS)),
        ("ranged_line_tokens", i64::from(RANGED_LINE_TOKENS)),
        ("orbit_step", i64::from(agents::ORBIT_STEP)),
        ("boss_arrival_above", i64::from(horde::ABOVE)),
        ("default_xp", i64::from(levels::DEFAULT_XP)),
        ("offer_size", stars::OFFER_SIZE as i64),
        ("defence_k", i64::from(entity::DEFENCE_K as u32)),
        (
            "crit_damage_base_pct",
            i64::from(combat::CRIT_DAMAGE_BASE_PCT as u32),
        ),
    ];
    let object = file.as_object().unwrap();
    for (key, value) in &want {
        assert_eq!(
            object.get(*key).and_then(Value::as_i64),
            Some(*value),
            "{key}"
        );
    }
    assert_eq!(object.len(), want.len(), "no other key");
}
