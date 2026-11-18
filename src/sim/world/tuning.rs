//! The run's rules numbers, read once when the run is built.

use crate::fixed::Fx;
use crate::rules::Rules;

/// The run's rules numbers (CS1b-3): the designer-facing fields of the pack's `rules.json`, read
/// once when the run is built, and the spawn rectangle derived from the view and the margin. A
/// `Copy` struct of plain values, so a reader costs a field read and allocates nothing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tuning {
    pub player_speed: i32,
    pub player_radius: i32,
    pub orb_radius: i32,
    pub hit_invulnerable_ticks: u32,
    pub aggro_per_damage: u32,
    pub aggro_per_buff: u32,
    pub aggro_per_heal_point: u32,
    pub aggro_decay: u32,
    /// How far a regular enemy may stray before the leash moves it, and where it lands (EC2, DS80).
    pub leash_reach_x: Fx,
    pub leash_reach_y: Fx,
    pub leash_land_x: Fx,
    pub leash_land_y: Fx,
    /// The spawn rectangle's half width (VS1, DS109) and half height: the view (`VIEW_HALF_WIDTH`, `VIEW_HALF_HEIGHT`) and the file's margin.
    pub spawn_edge_x: Fx,
    pub spawn_edge_y: Fx,
    pub ranged_token_cooldown_ticks: u32,
    pub ranged_zone_tokens: u32,
    pub ranged_line_tokens: u32,
    pub orbit_step: i32,
    pub boss_arrival_above: i32,
    pub default_xp: u32,
    pub defence_k: u64,
    pub crit_damage_base_pct: u64,
    pub party_damage_pct: [u32; crate::id::MAX_PLAYERS as usize],
    /// The poise damage of an A step's and of a smash step's combo hit that names none, the
    /// flinch's ticks and the immunity after it (0: no enemy flinches), `sim_version` 2 only.
    pub poise_damage_a: u32,
    pub poise_damage_b: u32,
    pub flinch_ticks: u16,
    pub flinch_immune_ticks: u16,
    /// What percent of its ticks the n-th hard control on one enemy keeps; `control_repeat_len`
    /// entries are used, 0 when no list is named (today's stun immunity alone).
    pub control_repeat_pcts: [u8; crate::limits::MAX_CONTROL_REPEATS as usize],
    pub control_repeat_len: u8,
    pub control_reset_ticks: u16,
    pub control_immune_ticks: u32,
    /// The `vulnerable` percent a stun that lands puts on its target; 0: none.
    pub vulnerable_stun_pct: u16,
    /// The perfect evade (`sim_version` 2); `None`: the rule is off.
    pub perfect_evade: Option<crate::rules::PerfectEvade>,
    /// The rally (`sim_version` 2); `None`: the rule is off.
    pub rally: Option<crate::rules::Rally>,
    /// The stagger gauge (`sim_version` 2; each 0 when its key is absent): what a smash step adds
    /// when its phase names none, what a resisted `[stun, pull, knockback]` adds, the percent of a
    /// fill kept by bars broken (`stagger_refill_len` entries, 0: all in full), the ticks before
    /// an idle gauge drains and what it loses a tick, the damage percent a staggered target takes
    /// more, the ticks of a stagger an agent names none of, and the `vulnerable` percent it puts.
    pub stagger_smash: u32,
    pub stagger_values: [u32; 3],
    pub stagger_refill_pcts: [u8; crate::limits::MAX_STAGGER_REFILLS as usize],
    pub stagger_refill_len: u8,
    pub stagger_drain_delay: u16,
    pub stagger_drain_per_tick: u32,
    pub stagger_damage_pct: u32,
    pub staggered_ticks: u16,
    pub vulnerable_stagger_pct: u16,
    /// The orb clear's radius in 1/256 px, clamped to the cap; 0: no clear (`sim_version` 2).
    pub orb_clear_radius: i32,
    /// The knockdown: the ticks a smash's knockback keeps a regular enemy down (0: no knockdown),
    /// the percent a smash hit on it deals more, the `vulnerable` percent it puts, and what a
    /// resisted knockback adds to a stagger gauge for it.
    pub down_ticks: u16,
    pub down_bonus_pct: u32,
    pub vulnerable_down_pct: u16,
    pub stagger_down: u32,
    /// The ticks a counter stuns a regular enemy or an elite, and what it adds to the gauge of an
    /// elite or the boss; each 0 when the rules do not name it.
    pub counter_stun_ticks: u32,
    pub stagger_counter: u32,
    /// The ticks a hit on a player keeps a regular enemy's ranged start off it (the grace); 0
    /// when the rules do not name it.
    pub ranged_grace_ticks: u16,
}

impl Tuning {
    pub(super) fn of(rules: &Rules) -> Tuning {
        let margin = rules.spawn_margin();
        let mut stagger_refill_pcts = [0u8; crate::limits::MAX_STAGGER_REFILLS as usize];
        for (to, from) in stagger_refill_pcts
            .iter_mut()
            .zip(rules.stagger_refill_pcts())
        {
            // Validated 1 to 100.
            *to = u8::try_from(*from).unwrap_or(100);
        }
        let mut control_repeat_pcts = [0u8; crate::limits::MAX_CONTROL_REPEATS as usize];
        for (to, from) in control_repeat_pcts
            .iter_mut()
            .zip(rules.control_repeat_pcts())
        {
            // Validated 1 to 100.
            *to = u8::try_from(*from).unwrap_or(100);
        }
        Tuning {
            player_speed: rules.player_speed().0,
            player_radius: rules.player_radius().0,
            orb_radius: rules.orb_radius().0,
            hit_invulnerable_ticks: rules.hit_invulnerable_ticks(),
            aggro_per_damage: rules.aggro_per_damage(),
            aggro_per_buff: rules.aggro_per_buff(),
            aggro_per_heal_point: rules.aggro_per_heal_point(),
            aggro_decay: rules.aggro_decay(),
            leash_reach_x: rules.leash_reach_x(),
            leash_reach_y: rules.leash_reach_y(),
            leash_land_x: rules.leash_land_x(),
            leash_land_y: rules.leash_land_y(),
            spawn_edge_x: Fx(super::VIEW_HALF_WIDTH.0 + margin.0),
            spawn_edge_y: Fx(super::VIEW_HALF_HEIGHT.0 + margin.0),
            ranged_token_cooldown_ticks: rules.ranged_token_cooldown_ticks(),
            ranged_zone_tokens: rules.ranged_zone_tokens(),
            ranged_line_tokens: rules.ranged_line_tokens(),
            orbit_step: i32::try_from(rules.orbit_step()).unwrap_or(i32::MAX),
            boss_arrival_above: rules.boss_arrival_above().0,
            default_xp: rules.default_xp(),
            defence_k: u64::from(rules.defence_k()),
            crit_damage_base_pct: u64::from(rules.crit_damage_base_pct()),
            party_damage_pct: rules.party_damage_pct(),
            poise_damage_a: rules.poise_damage_a(),
            poise_damage_b: rules.poise_damage_b(),
            flinch_ticks: rules.flinch_ticks(),
            flinch_immune_ticks: rules.flinch_immune_ticks(),
            control_repeat_pcts,
            control_repeat_len: u8::try_from(rules.control_repeat_pcts().len()).unwrap_or(0),
            control_reset_ticks: rules.control_reset_ticks(),
            control_immune_ticks: rules.control_immune_ticks(),
            vulnerable_stun_pct: u16::try_from(rules.vulnerable_stun_pct()).unwrap_or(u16::MAX),
            perfect_evade: rules.perfect_evade(),
            rally: rules.rally(),
            stagger_smash: rules.stagger_smash(),
            stagger_values: rules.stagger_values(),
            stagger_refill_pcts,
            stagger_refill_len: u8::try_from(rules.stagger_refill_pcts().len()).unwrap_or(0),
            stagger_drain_delay: rules.stagger_drain().0,
            stagger_drain_per_tick: rules.stagger_drain().1,
            stagger_damage_pct: rules.stagger_damage_pct(),
            staggered_ticks: rules.staggered_ticks(),
            vulnerable_stagger_pct: u16::try_from(rules.vulnerable_stagger_pct())
                .unwrap_or(u16::MAX),
            orb_clear_radius: rules.orb_clear_radius().map_or(0, |r| r.0),
            down_ticks: rules.down_ticks(),
            down_bonus_pct: rules.down_bonus_pct(),
            vulnerable_down_pct: u16::try_from(rules.vulnerable_knockdown_pct())
                .unwrap_or(u16::MAX),
            stagger_down: rules.knockdown_stagger_value(),
            counter_stun_ticks: rules.counter_stun_ticks(),
            stagger_counter: rules.counter_stagger_value(),
            ranged_grace_ticks: rules.ranged_grace_ticks(),
        }
    }
}
