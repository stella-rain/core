//! One player's body and the states that hang off it (ADR-SIM-3, ADR-SIM-7).

use crate::attack::DamageType;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::rune::Rune;

use super::{Dir, Statuses};

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PlayerState {
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears.
    pub defence: u32,
    /// Ticks of invulnerability left, after a hit or from an evade.
    pub invulnerable: u32,
    /// The last non-zero movement direction: where a combo step attacks and where an
    /// evade without movement dashes.
    pub facing: Dir,
    /// The enemy aim assist and auto mode face, picked each tick while either is on.
    pub target: Option<EntityId>,
    /// The evade's charges held, ready or recovering.
    pub evade_charges: u8,
    /// Ticks left on each spent charge, in the order they were spent; none is 0 and there are
    /// never more than `evade_charges`.
    pub evade_recovering: Vec<u32>,
    /// The buttons held on the last tick (`input::held_buttons`), so that only a button held now
    /// and not then is a press.
    pub held: u8,
    /// Where it is in its Main weapon's combos.
    pub combo: ComboState,
    /// The dash in progress, if any.
    pub dash: Option<DashState>,
    /// The stage skill slot: empty until a reward fills it.
    pub skill: Option<SkillSlot>,
    /// The stage skill's pull while it pulls, and its shield while it holds.
    pub pull: Option<PullState>,
    pub shield: Option<ShieldState>,
    /// The orbits rewards gave, in the order they came, at most `MAX_ORBITS`.
    pub orbits: Vec<OrbitState>,
    /// What the range rewards taken add up to, in percent, saturating.
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, saturating.
    pub rune_haste_pct: u32,
    /// What the pickup radius rewards taken add up to, in percent, saturating: the pickup
    /// circle of stars and heal pickups is that much wider, held to
    /// `MAX_PICKUP_RADIUS_TOTAL_PCT` where it is used.
    pub pickup_radius_pct: u32,
    /// What the skill cooldown rewards taken add up to, in percent, saturating: the stage
    /// skill's cooldown is that much shorter, held to `MAX_SKILL_COOLDOWN_TOTAL_PCT`.
    pub skill_cooldown_pct: u32,
    /// Hit points (the stage's scale) the heal-per-kills rewards taken heal together each time
    /// `kill_heal_every` kills have been counted; their amounts add up, held to
    /// `MAX_HEAL_AMOUNT` where it is used.
    pub kill_heal: u32,
    /// The kills between two such heals: the shortest interval among the picks, 0 while none
    /// is held.
    pub kill_heal_every: u32,
    /// Kills counted since the last such heal, below `kill_heal_every` between ticks.
    pub kills_counted: u32,
    /// What the crit chance rewards taken add up to, in percent, saturating; held to
    /// `MAX_CRIT_CHANCE_TOTAL_PCT` where it is rolled.
    pub crit_chance_pct: u32,
    /// What the crit damage rewards add up to: a crit does twice the damage and this many
    /// percent more, held to `MAX_CRIT_DAMAGE_TOTAL_PCT`.
    pub crit_damage_pct: u32,
    /// What the echo rewards add up to: each hit echoes this percent of its damage, held to
    /// `MAX_ECHO_HIT_PCT`; 0 while none is held.
    pub echo_hit_pct: u32,
    /// The kill burst rewards taken, added up: the percent of the dead enemy's maximum hit
    /// points, the radius in pixels, and the longest chain; all 0 while none is held.
    pub burst_pct: u32,
    pub burst_radius: u32,
    pub burst_depth: u32,
    /// Links more on the chain, from `burst_depth` rewards.
    pub burst_depth_extra: u32,
    /// What the type bonus rewards taken add up to for each damage type, by its place in
    /// `DamageType::ALL`, in percent, saturating: a group's bonus is added to each
    /// type of its group.
    pub type_bonus_pct: [u32; DamageType::ALL.len()],
    /// The runes the Main weapon holds, in the order drawn, each once, at most `MAX_RUNES`
    ///.
    pub runes: Vec<HeldRune>,
    pub aggro: u32,
    pub statuses: Statuses,
    /// Ticks elapsed since downed (0 while up or out, ADR-SIM-7).
    pub down_ticks: u32,
    /// Ticks accumulated toward revive while interact is held in range (0 while up or out, ADR-SIM-7).
    pub revive_hold: u32,
    /// Whether the player is permanently out for the run (ADR-SIM-7).
    pub out: bool,
    /// The combat rules' counters; all zero until a rule writes them.
    pub combat: PlayerCombat,
}

/// A player's combat counters (ADR-SIM-5 point 9: each is capped at decode by its `limits.rs`
/// constant). Ticks left unless the name says otherwise; zero is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlayerCombat {
    /// Ticks left of the perfect-evade window.
    pub perfect_window: u16,
    /// Ticks left of the slow a perfect evade gives.
    pub perfect_slow: u16,
    /// Ticks left before another perfect evade slows.
    pub perfect_cooldown: u16,
    /// The weapon gauge, and what one step has filled into it.
    pub weapon_gauge: u32,
    pub gauge_step_fill: u32,
    /// Hit points that can be won back by hitting, and the ticks left before they lapse.
    pub rally: u32,
    pub rally_ticks: u16,
    /// The spirit level and the ticks left before it decays.
    pub spirit_level: u8,
    pub spirit_decay: u16,
    /// Ticks left of the counter window.
    pub counter_window: u16,
    /// The combo tempo stacks held.
    pub combo_tempo: u8,
    /// Ticks left before the giant trigger can fire again.
    pub giant_trigger: u16,
    /// Ticks left in which a ranged start is refused against this player as its target.
    pub ranged_grace: u16,
}

impl Clone for PlayerState {
    fn clone(&self) -> PlayerState {
        let PlayerState {
            at,
            hp,
            max_hp,
            defence,
            invulnerable,
            facing,
            target,
            evade_charges,
            evade_recovering,
            held,
            combo,
            dash,
            skill,
            pull,
            shield,
            orbits,
            range_pct,
            rune_haste_pct,
            pickup_radius_pct,
            skill_cooldown_pct,
            kill_heal,
            kill_heal_every,
            kills_counted,
            crit_chance_pct,
            crit_damage_pct,
            echo_hit_pct,
            burst_pct,
            burst_radius,
            burst_depth,
            burst_depth_extra,
            type_bonus_pct,
            runes,
            aggro,
            statuses,
            down_ticks,
            revive_hold,
            out,
            combat,
        } = self;
        PlayerState {
            at: *at,
            hp: *hp,
            max_hp: *max_hp,
            defence: *defence,
            invulnerable: *invulnerable,
            facing: *facing,
            target: *target,
            evade_charges: *evade_charges,
            evade_recovering: evade_recovering.clone(),
            held: *held,
            combo: *combo,
            dash: *dash,
            skill: *skill,
            pull: *pull,
            shield: *shield,
            orbits: orbits.clone(),
            range_pct: *range_pct,
            rune_haste_pct: *rune_haste_pct,
            pickup_radius_pct: *pickup_radius_pct,
            skill_cooldown_pct: *skill_cooldown_pct,
            kill_heal: *kill_heal,
            kill_heal_every: *kill_heal_every,
            kills_counted: *kills_counted,
            crit_chance_pct: *crit_chance_pct,
            crit_damage_pct: *crit_damage_pct,
            echo_hit_pct: *echo_hit_pct,
            burst_pct: *burst_pct,
            burst_radius: *burst_radius,
            burst_depth: *burst_depth,
            burst_depth_extra: *burst_depth_extra,
            type_bonus_pct: *type_bonus_pct,
            runes: runes.clone(),
            aggro: *aggro,
            statuses: *statuses,
            down_ticks: *down_ticks,
            revive_hold: *revive_hold,
            out: *out,
            combat: *combat,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &PlayerState) {
        let PlayerState {
            at,
            hp,
            max_hp,
            defence,
            invulnerable,
            facing,
            target,
            evade_charges,
            evade_recovering,
            held,
            combo,
            dash,
            skill,
            pull,
            shield,
            orbits,
            range_pct,
            rune_haste_pct,
            pickup_radius_pct,
            skill_cooldown_pct,
            kill_heal,
            kill_heal_every,
            kills_counted,
            crit_chance_pct,
            crit_damage_pct,
            echo_hit_pct,
            burst_pct,
            burst_radius,
            burst_depth,
            burst_depth_extra,
            type_bonus_pct,
            runes,
            aggro,
            statuses,
            down_ticks,
            revive_hold,
            out,
            combat,
        } = self;
        let PlayerState {
            at: s_at,
            hp: s_hp,
            max_hp: s_max_hp,
            defence: s_defence,
            invulnerable: s_invulnerable,
            facing: s_facing,
            target: s_target,
            evade_charges: s_evade_charges,
            evade_recovering: s_evade_recovering,
            held: s_held,
            combo: s_combo,
            dash: s_dash,
            skill: s_skill,
            pull: s_pull,
            shield: s_shield,
            orbits: s_orbits,
            range_pct: s_range_pct,
            rune_haste_pct: s_rune_haste_pct,
            pickup_radius_pct: s_pickup_radius_pct,
            skill_cooldown_pct: s_skill_cooldown_pct,
            kill_heal: s_kill_heal,
            kill_heal_every: s_kill_heal_every,
            kills_counted: s_kills_counted,
            crit_chance_pct: s_crit_chance_pct,
            crit_damage_pct: s_crit_damage_pct,
            echo_hit_pct: s_echo_hit_pct,
            burst_pct: s_burst_pct,
            burst_radius: s_burst_radius,
            burst_depth: s_burst_depth,
            burst_depth_extra: s_burst_depth_extra,
            type_bonus_pct: s_type_bonus_pct,
            runes: s_runes,
            aggro: s_aggro,
            statuses: s_statuses,
            down_ticks: s_down_ticks,
            revive_hold: s_revive_hold,
            out: s_out,
            combat: s_combat,
        } = source;
        *at = *s_at;
        *hp = *s_hp;
        *max_hp = *s_max_hp;
        *defence = *s_defence;
        *invulnerable = *s_invulnerable;
        *facing = *s_facing;
        *target = *s_target;
        *evade_charges = *s_evade_charges;
        evade_recovering.clone_from(s_evade_recovering);
        *held = *s_held;
        *combo = *s_combo;
        *dash = *s_dash;
        *skill = *s_skill;
        *pull = *s_pull;
        *shield = *s_shield;
        orbits.clone_from(s_orbits);
        *range_pct = *s_range_pct;
        *rune_haste_pct = *s_rune_haste_pct;
        *pickup_radius_pct = *s_pickup_radius_pct;
        *skill_cooldown_pct = *s_skill_cooldown_pct;
        *kill_heal = *s_kill_heal;
        *kill_heal_every = *s_kill_heal_every;
        *kills_counted = *s_kills_counted;
        *crit_chance_pct = *s_crit_chance_pct;
        *crit_damage_pct = *s_crit_damage_pct;
        *echo_hit_pct = *s_echo_hit_pct;
        *burst_pct = *s_burst_pct;
        *burst_radius = *s_burst_radius;
        *burst_depth = *s_burst_depth;
        *burst_depth_extra = *s_burst_depth_extra;
        *type_bonus_pct = *s_type_bonus_pct;
        runes.clone_from(s_runes);
        *aggro = *s_aggro;
        *statuses = *s_statuses;
        *down_ticks = *s_down_ticks;
        *revive_hold = *s_revive_hold;
        *out = *s_out;
        *combat = *s_combat;
    }
}

impl PlayerState {
    /// Alive and not out (ADR-SIM-7).
    #[inline]
    pub fn is_up(&self) -> bool {
        self.hp > 0 && !self.is_out()
    }

    /// At 0 HP and not yet permanently out (ADR-SIM-7).
    #[inline]
    pub fn is_downed(&self) -> bool {
        self.hp == 0 && !self.is_out()
    }

    /// Permanently out for the run (ADR-SIM-7).
    #[inline]
    pub fn is_out(&self) -> bool {
        self.out
    }

    /// The evade's charges ready: held and not recovering.
    pub fn evade_ready(&self) -> u8 {
        // There are never more timers than charges, and charges are at most 3.
        self.evade_charges
            .saturating_sub(u8::try_from(self.evade_recovering.len()).expect("at most 3 charges"))
    }
}

/// Where the main character is in its Main weapon's combo tree.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ComboState {
    /// The presses from the root to the current step, the first in bit 0: 1 for `smash`.
    pub path: u32,
    /// How many presses `path` holds: 0 at the root.
    pub depth: u8,
    /// The path began in the dash branch.
    pub dash: bool,
    /// The press waiting for the next step: `Some(true)` for `smash`.
    pub buffered: Option<bool>,
    /// The current step's attack is running.
    pub running: bool,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
    /// Ticks left in which a press from the root takes the dash branch.
    pub dash_strike_left: u32,
    /// The current step charging: ticks held, 0 on the tick it began.
    pub charge: Option<u32>,
    /// The current step held: ticks held, 0 on the tick it began.
    pub hold: Option<u32>,
    /// The current step's hold has released, so its release attack, and its recovery, never
    /// begins a hold again; a step taken next clears it.
    pub released: bool,
    /// A press by hand took a step in this combo: auto mode waits, and the combo
    /// becomes the auto combo when it resets, unless it began in the dash branch.
    pub hand: bool,
    /// The auto combo auto mode plays: its presses from the root, the first in bit 0 (1 for
    /// `smash`), and how many; at most the tree's depth.
    pub auto_path: u32,
    pub auto_depth: u8,
    /// The charge level each step of the combo was released at, 4 bits a step from
    /// the root, the first in the low bits (a charge has at most `MAX_CHARGE_LEVELS` = 8
    /// levels): by hand, the level the player let go at; for a step auto mode played, the level
    /// it played. 0 for a plain step.
    pub levels: u32,
    /// The same for the auto combo: the level auto mode charges each of its steps to.
    pub auto_levels: u32,
    /// The current charge is auto mode's: it needs no button and is never perfect.
    pub charge_auto: bool,
}

/// The Sub weapon's pull on what a combo part hits, carried by the attack and the
/// orbs it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
}

/// What a smash (a step reached by B) does to the enemy it hits, carried by
/// the attack and the orbs it spawns, as a combo step's hits carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Smash {
    Pull(Pull),
    /// The enemy is pushed away from the main character, as far and as long as the pull's numbers.
    Knockback(Pull),
    /// The `bleed` status, with the bleed rune's numbers from the runes file.
    Bleed,
    /// The `stun` status of a smash, for its ticks.
    Stun {
        ticks: u32,
    },
    /// The `reap` smash: a hit adds a bleed stack, and a hit on an enemy at `stacks` cashes them;
    /// a boss takes `boss_pct` percent of the burst (ADR-SIM-9).
    Reap {
        stacks: u32,
        boss_pct: u32,
    },
}

/// The stage skill held: its place in the stage skills file, its level from 1, and
/// the ticks until it is ready again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SkillSlot {
    pub index: u16,
    pub level: u8,
    pub cooldown_left: u32,
}

/// An orbit held: its reward's place in the rewards file, its orbs, the orbs of the round going
/// on, the ticks until the next round may start, and where orb 0 is round the turn (1/256
/// degree from straight down the screen, in `0..PER_TURN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbitState {
    pub reward: u16,
    pub orbs: u8,
    pub round: u8,
    pub cooldown_left: u32,
    pub angle: i32,
}

/// A rune the Main weapon holds, and for an on-hit rune the ticks until it may fire again;
/// always 0 for a damage-over-time rune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeldRune {
    pub rune: Rune,
    pub cooldown_left: u32,
}

/// A pull pulling: its point, radius and strength (fixed-point units), and the ticks left, at
/// least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PullState {
    pub at: Point,
    pub radius: i32,
    pub strength: i32,
    pub ticks_left: u32,
}

/// A shield: the damage it still absorbs, at least 1, and the ticks left, at least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ShieldState {
    pub amount: u32,
    pub ticks_left: u32,
}

/// An evade's dash: locked to its direction, `done` of its ticks moved, `left` to go.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct DashState {
    pub dir: Dir,
    pub done: u32,
    pub left: u32,
}
