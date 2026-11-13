//! The savepoint encoding of one player's state.

use super::shared::cap_u32;
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec};
use crate::sim::state::*;

impl Encode for Pull {
    fn encode(&self, w: &mut Writer) {
        let Pull { distance, ticks } = self;
        distance.encode(w);
        ticks.encode(w);
    }
}

impl Decode for Pull {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Pull {
            distance: Decode::decode(r)?,
            ticks: Decode::decode(r)?,
        })
    }
}

impl Encode for Smash {
    fn encode(&self, w: &mut Writer) {
        match self {
            Smash::Pull(pull) => {
                w.u8(0);
                pull.encode(w);
            }
            Smash::Knockback(pull) => {
                w.u8(1);
                pull.encode(w);
            }
            Smash::Bleed => w.u8(2),
            Smash::Stun { ticks } => {
                w.u8(3);
                ticks.encode(w);
            }
            Smash::Reap { stacks, boss_pct } => {
                w.u8(4);
                stacks.encode(w);
                boss_pct.encode(w);
            }
        }
    }
}

impl Decode for Smash {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(Smash::Pull(Decode::decode(r)?)),
            1 => Ok(Smash::Knockback(Decode::decode(r)?)),
            2 => Ok(Smash::Bleed),
            3 => Ok(Smash::Stun {
                ticks: Decode::decode(r)?,
            }),
            4 => Ok(Smash::Reap {
                stacks: Decode::decode(r)?,
                boss_pct: Decode::decode(r)?,
            }),
            _ => Err(DecodeError::Invalid("smash tag")),
        }
    }
}

impl Encode for PlayerState {
    fn encode(&self, w: &mut Writer) {
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
        at.encode(w);
        hp.encode(w);
        max_hp.encode(w);
        defence.encode(w);
        invulnerable.encode(w);
        facing.encode(w);
        target.encode(w);
        evade_charges.encode(w);
        evade_recovering.encode(w);
        held.encode(w);
        combo.encode(w);
        dash.encode(w);
        skill.encode(w);
        pull.encode(w);
        shield.encode(w);
        orbits.encode(w);
        range_pct.encode(w);
        rune_haste_pct.encode(w);
        pickup_radius_pct.encode(w);
        skill_cooldown_pct.encode(w);
        kill_heal.encode(w);
        kill_heal_every.encode(w);
        kills_counted.encode(w);
        crit_chance_pct.encode(w);
        crit_damage_pct.encode(w);
        echo_hit_pct.encode(w);
        burst_pct.encode(w);
        burst_radius.encode(w);
        burst_depth.encode(w);
        burst_depth_extra.encode(w);
        type_bonus_pct.encode(w);
        runes.encode(w);
        aggro.encode(w);
        statuses.encode(w);
        down_ticks.encode(w);
        revive_hold.encode(w);
        out.encode(w);
        combat.encode(w);
    }
}

impl Decode for PlayerState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let player = PlayerState {
            at: Decode::decode(r)?,
            hp: Decode::decode(r)?,
            max_hp: Decode::decode(r)?,
            defence: Decode::decode(r)?,
            invulnerable: Decode::decode(r)?,
            facing: Decode::decode(r)?,
            target: Decode::decode(r)?,
            evade_charges: Decode::decode(r)?,
            evade_recovering: decode_vec(
                r,
                cap_u32(usize::from(crate::validate::limits::MAX_EVADE_CHARGES)),
            )?,
            held: Decode::decode(r)?,
            combo: Decode::decode(r)?,
            dash: Decode::decode(r)?,
            skill: Decode::decode(r)?,
            pull: Decode::decode(r)?,
            shield: Decode::decode(r)?,
            orbits: decode_vec(r, cap_u32(crate::rewards::MAX_ORBITS))?,
            range_pct: Decode::decode(r)?,
            rune_haste_pct: Decode::decode(r)?,
            pickup_radius_pct: Decode::decode(r)?,
            skill_cooldown_pct: Decode::decode(r)?,
            kill_heal: Decode::decode(r)?,
            kill_heal_every: Decode::decode(r)?,
            kills_counted: Decode::decode(r)?,
            crit_chance_pct: Decode::decode(r)?,
            crit_damage_pct: Decode::decode(r)?,
            echo_hit_pct: Decode::decode(r)?,
            burst_pct: Decode::decode(r)?,
            burst_radius: Decode::decode(r)?,
            burst_depth: Decode::decode(r)?,
            burst_depth_extra: Decode::decode(r)?,
            type_bonus_pct: Decode::decode(r)?,
            runes: decode_vec(r, cap_u32(crate::rune::MAX_RUNES))?,
            aggro: Decode::decode(r)?,
            statuses: Decode::decode(r)?,
            down_ticks: Decode::decode(r)?,
            revive_hold: Decode::decode(r)?,
            out: Decode::decode(r)?,
            combat: Decode::decode(r)?,
        };
        // The hit points a rally can win back never pass the player's own maximum.
        if player.combat.rally > player.max_hp {
            return Err(DecodeError::Invalid("rally past its bound"));
        }
        Ok(player)
    }
}

impl Encode for PlayerCombat {
    fn encode(&self, w: &mut Writer) {
        let PlayerCombat {
            perfect_window,
            perfect_slow,
            perfect_cooldown,
            weapon_gauge,
            gauge_step_fill,
            rally,
            rally_ticks,
            spirit_level,
            spirit_decay,
            counter_window,
            combo_tempo,
            giant_trigger,
            ranged_grace,
            counter_bonus_step,
        } = self;
        perfect_window.encode(w);
        perfect_slow.encode(w);
        perfect_cooldown.encode(w);
        weapon_gauge.encode(w);
        gauge_step_fill.encode(w);
        rally.encode(w);
        rally_ticks.encode(w);
        spirit_level.encode(w);
        spirit_decay.encode(w);
        counter_window.encode(w);
        combo_tempo.encode(w);
        giant_trigger.encode(w);
        ranged_grace.encode(w);
        counter_bonus_step.encode(w);
    }
}

impl Decode for PlayerCombat {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        use super::foes::within;
        use crate::limits::*;
        Ok(PlayerCombat {
            perfect_window: within(
                r.u16()?,
                MAX_PERFECT_WINDOW_TICKS,
                "perfect_window past its bound",
            )?,
            perfect_slow: within(
                r.u16()?,
                MAX_PERFECT_SLOW_TICKS,
                "perfect_slow past its bound",
            )?,
            perfect_cooldown: within(
                r.u16()?,
                MAX_PERFECT_COOLDOWN_TICKS,
                "perfect_cooldown past its bound",
            )?,
            weapon_gauge: within(r.u32()?, MAX_WEAPON_GAUGE, "weapon_gauge past its bound")?,
            gauge_step_fill: within(r.u32()?, MAX_WEAPON_GAUGE, "gauge_step_fill past its bound")?,
            rally: within(r.u32()?, MAX_RALLY, "rally past its bound")?,
            rally_ticks: within(r.u16()?, MAX_RALLY_TICKS, "rally_ticks past its bound")?,
            spirit_level: within(r.u8()?, MAX_SPIRIT_LEVEL, "spirit_level past its bound")?,
            spirit_decay: within(
                r.u16()?,
                MAX_SPIRIT_DECAY_TICKS,
                "spirit_decay past its bound",
            )?,
            counter_window: within(
                r.u16()?,
                MAX_COUNTER_WINDOW_TICKS,
                "counter_window past its bound",
            )?,
            combo_tempo: within(r.u8()?, MAX_TEMPO_STACKS, "combo_tempo past its bound")?,
            giant_trigger: within(r.u16()?, MAX_TRIGGER_TICKS, "giant_trigger past its bound")?,
            ranged_grace: within(
                r.u16()?,
                MAX_RANGED_GRACE_TICKS,
                "ranged_grace past its bound",
            )?,
            counter_bonus_step: within(
                r.u8()?,
                MAX_COUNTER_BONUS_STEP,
                "counter_bonus_step past its bound",
            )?,
        })
    }
}

impl Encode for ComboState {
    fn encode(&self, w: &mut Writer) {
        let ComboState {
            path,
            depth,
            dash,
            buffered,
            running,
            window_left,
            dash_strike_left,
            charge,
            hold,
            released,
            hand,
            auto_path,
            auto_depth,
            levels,
            auto_levels,
            charge_auto,
        } = self;
        path.encode(w);
        depth.encode(w);
        dash.encode(w);
        buffered.encode(w);
        running.encode(w);
        window_left.encode(w);
        dash_strike_left.encode(w);
        charge.encode(w);
        hold.encode(w);
        released.encode(w);
        hand.encode(w);
        auto_path.encode(w);
        auto_depth.encode(w);
        levels.encode(w);
        auto_levels.encode(w);
        charge_auto.encode(w);
    }
}

impl Decode for ComboState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ComboState {
            path: Decode::decode(r)?,
            depth: Decode::decode(r)?,
            dash: Decode::decode(r)?,
            buffered: Decode::decode(r)?,
            running: Decode::decode(r)?,
            window_left: Decode::decode(r)?,
            dash_strike_left: Decode::decode(r)?,
            charge: Decode::decode(r)?,
            hold: Decode::decode(r)?,
            released: Decode::decode(r)?,
            hand: Decode::decode(r)?,
            auto_path: Decode::decode(r)?,
            auto_depth: Decode::decode(r)?,
            levels: Decode::decode(r)?,
            auto_levels: Decode::decode(r)?,
            charge_auto: Decode::decode(r)?,
        })
    }
}

impl Encode for SkillSlot {
    fn encode(&self, w: &mut Writer) {
        let SkillSlot {
            index,
            level,
            cooldown_left,
        } = self;
        index.encode(w);
        level.encode(w);
        cooldown_left.encode(w);
    }
}

impl Decode for SkillSlot {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(SkillSlot {
            index: Decode::decode(r)?,
            level: Decode::decode(r)?,
            cooldown_left: Decode::decode(r)?,
        })
    }
}

impl Encode for OrbitState {
    fn encode(&self, w: &mut Writer) {
        let OrbitState {
            reward,
            orbs,
            round,
            cooldown_left,
            angle,
        } = self;
        reward.encode(w);
        orbs.encode(w);
        round.encode(w);
        cooldown_left.encode(w);
        angle.encode(w);
    }
}

impl Decode for OrbitState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(OrbitState {
            reward: Decode::decode(r)?,
            orbs: Decode::decode(r)?,
            round: Decode::decode(r)?,
            cooldown_left: Decode::decode(r)?,
            angle: Decode::decode(r)?,
        })
    }
}

impl Encode for HeldRune {
    fn encode(&self, w: &mut Writer) {
        let HeldRune {
            rune,
            cooldown_left,
        } = self;
        rune.encode(w);
        cooldown_left.encode(w);
    }
}

impl Decode for HeldRune {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(HeldRune {
            rune: Decode::decode(r)?,
            cooldown_left: Decode::decode(r)?,
        })
    }
}

impl Encode for PullState {
    fn encode(&self, w: &mut Writer) {
        let PullState {
            at,
            radius,
            strength,
            ticks_left,
        } = self;
        at.encode(w);
        radius.encode(w);
        strength.encode(w);
        ticks_left.encode(w);
    }
}

impl Decode for PullState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(PullState {
            at: Decode::decode(r)?,
            radius: Decode::decode(r)?,
            strength: Decode::decode(r)?,
            ticks_left: Decode::decode(r)?,
        })
    }
}

impl Encode for ShieldState {
    fn encode(&self, w: &mut Writer) {
        let ShieldState { amount, ticks_left } = self;
        amount.encode(w);
        ticks_left.encode(w);
    }
}

impl Decode for ShieldState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ShieldState {
            amount: Decode::decode(r)?,
            ticks_left: Decode::decode(r)?,
        })
    }
}

impl Encode for DashState {
    fn encode(&self, w: &mut Writer) {
        let DashState { dir, done, left } = self;
        dir.encode(w);
        done.encode(w);
        left.encode(w);
    }
}

impl Decode for DashState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(DashState {
            dir: Decode::decode(r)?,
            done: Decode::decode(r)?,
            left: Decode::decode(r)?,
        })
    }
}
