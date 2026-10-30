//! The savepoint encoding of every `State` type, beside the hash in `state_hash.rs` and in the
//! same order: each impl destructures its type without `..`, so a field added without a decision
//! does not compile. Every field is saved, the content indices the hash skips included, because
//! they are what a restored run reads; the pack hash guards that they mean the same content.
//!
//! The layout is the fields in declaration order, an enum a `u8` tag (the number its
//! `StableHash` writes), an `Option` a `0`/`1` tag, a list a `u32` length (see `savepoint`).

use crate::savepoint::codec::{
    Decode, DecodeError, Encode, Reader, Writer, decode_vec, max_guardians, max_hits, max_parts,
    max_path,
};

use super::state::*;
use super::world::RangedPool;
use crate::engine::Outcome;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::snapshot::{ExpertiseOfferKind, OfferKind};

// ---- Attacks, orbs and zones ----

impl Encode for Dir {
    fn encode(&self, w: &mut Writer) {
        let Dir { x, y } = self;
        x.encode(w);
        y.encode(w);
    }
}

impl Decode for Dir {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Dir {
            x: Decode::decode(r)?,
            y: Decode::decode(r)?,
        })
    }
}

impl Encode for PathStep {
    fn encode(&self, w: &mut Writer) {
        let PathStep { node, then } = self;
        node.encode(w);
        then.encode(w);
    }
}

impl Decode for PathStep {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(PathStep {
            node: Decode::decode(r)?,
            then: Decode::decode(r)?,
        })
    }
}

impl Encode for Frame {
    fn encode(&self, w: &mut Writer) {
        let Frame {
            idx,
            loop_i,
            loop_n,
            param,
            offset,
        } = self;
        idx.encode(w);
        loop_i.encode(w);
        loop_n.encode(w);
        param.encode(w);
        offset.encode(w);
    }
}

impl Decode for Frame {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Frame {
            idx: Decode::decode(r)?,
            loop_i: Decode::decode(r)?,
            loop_n: Decode::decode(r)?,
            param: Decode::decode(r)?,
            offset: Decode::decode(r)?,
        })
    }
}

impl Encode for Aim {
    fn encode(&self, w: &mut Writer) {
        match self {
            Aim::Entity(id) => {
                w.u8(0);
                id.encode(w);
            }
            Aim::Fixed(dir) => {
                w.u8(1);
                dir.encode(w);
            }
        }
    }
}

impl Decode for Aim {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(Aim::Entity(Decode::decode(r)?)),
            1 => Ok(Aim::Fixed(Decode::decode(r)?)),
            _ => Err(DecodeError::Invalid("aim tag")),
        }
    }
}

impl Encode for Origin {
    fn encode(&self, w: &mut Writer) {
        match self {
            Origin::Owner => w.u8(0),
            Origin::At(at) => {
                w.u8(1);
                at.encode(w);
            }
        }
    }
}

impl Decode for Origin {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(Origin::Owner),
            1 => Ok(Origin::At(Decode::decode(r)?)),
            _ => Err(DecodeError::Invalid("origin tag")),
        }
    }
}

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
            _ => Err(DecodeError::Invalid("smash tag")),
        }
    }
}

impl Encode for AttackTask {
    fn encode(&self, w: &mut Writer) {
        let AttackTask {
            owner,
            program,
            base,
            origin,
            aim,
            friendly,
            damage,
            damage_type,
            breaks_charge,
            smash,
            combo,
            age,
            wait_left,
            frames,
            parent_dir,
            last_dir,
            spot,
        } = self;
        owner.encode(w);
        program.encode(w);
        base.encode(w);
        origin.encode(w);
        aim.encode(w);
        friendly.encode(w);
        damage.encode(w);
        damage_type.encode(w);
        breaks_charge.encode(w);
        smash.encode(w);
        combo.encode(w);
        age.encode(w);
        wait_left.encode(w);
        frames.encode(w);
        parent_dir.encode(w);
        last_dir.encode(w);
        spot.encode(w);
    }
}

impl Decode for AttackTask {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(AttackTask {
            owner: Decode::decode(r)?,
            program: Decode::decode(r)?,
            base: decode_vec(r, max_path())?,
            origin: Decode::decode(r)?,
            aim: Decode::decode(r)?,
            friendly: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
            breaks_charge: Decode::decode(r)?,
            smash: Decode::decode(r)?,
            combo: Decode::decode(r)?,
            age: Decode::decode(r)?,
            wait_left: Decode::decode(r)?,
            frames: Decode::decode(r)?,
            parent_dir: Decode::decode(r)?,
            last_dir: Decode::decode(r)?,
            spot: Decode::decode(r)?,
        })
    }
}

impl Encode for AttackRun {
    fn encode(&self, w: &mut Writer) {
        let AttackRun {
            owner,
            slot,
            attack,
            speed_pct,
            width_pct,
            phase,
            tick,
            facing,
            aim,
            friendly,
            ranged,
            base,
            atk_up_pct,
            damage_pct,
            type_bonus_pct,
            age,
            hits,
            smash,
            at,
            throw,
            homing,
            spot,
        } = self;
        owner.encode(w);
        slot.encode(w);
        attack.encode(w);
        speed_pct.encode(w);
        width_pct.encode(w);
        phase.encode(w);
        tick.encode(w);
        facing.encode(w);
        aim.encode(w);
        friendly.encode(w);
        ranged.encode(w);
        base.encode(w);
        atk_up_pct.encode(w);
        damage_pct.encode(w);
        type_bonus_pct.encode(w);
        age.encode(w);
        hits.encode(w);
        smash.encode(w);
        at.encode(w);
        throw.encode(w);
        homing.encode(w);
        spot.encode(w);
    }
}

impl Decode for AttackRun {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(AttackRun {
            owner: Decode::decode(r)?,
            slot: Decode::decode(r)?,
            attack: Decode::decode(r)?,
            speed_pct: Decode::decode(r)?,
            width_pct: Decode::decode(r)?,
            phase: Decode::decode(r)?,
            tick: Decode::decode(r)?,
            facing: Decode::decode(r)?,
            aim: Decode::decode(r)?,
            friendly: Decode::decode(r)?,
            ranged: Decode::decode(r)?,
            base: Decode::decode(r)?,
            atk_up_pct: Decode::decode(r)?,
            damage_pct: Decode::decode(r)?,
            type_bonus_pct: Decode::decode(r)?,
            age: Decode::decode(r)?,
            hits: decode_vec(r, max_hits())?,
            smash: Decode::decode(r)?,
            at: Decode::decode(r)?,
            throw: Decode::decode(r)?,
            homing: Decode::decode(r)?,
            spot: Decode::decode(r)?,
        })
    }
}

impl Encode for OrbLink {
    fn encode(&self, w: &mut Writer) {
        let OrbLink {
            group,
            slot,
            of,
            width,
            damage,
            rehit_ticks,
            last_hit,
        } = self;
        group.encode(w);
        slot.encode(w);
        of.encode(w);
        width.encode(w);
        damage.encode(w);
        rehit_ticks.encode(w);
        last_hit.encode(w);
    }
}

impl Decode for OrbLink {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(OrbLink {
            group: Decode::decode(r)?,
            slot: Decode::decode(r)?,
            of: Decode::decode(r)?,
            width: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            rehit_ticks: Decode::decode(r)?,
            last_hit: Decode::decode(r)?,
        })
    }
}

impl Encode for Hook {
    fn encode(&self, w: &mut Writer) {
        let Hook {
            program,
            path,
            left,
            owner,
            aim,
            damage,
            damage_type,
            breaks_charge,
            smash,
            combo,
        } = self;
        program.encode(w);
        path.encode(w);
        left.encode(w);
        owner.encode(w);
        aim.encode(w);
        damage.encode(w);
        damage_type.encode(w);
        breaks_charge.encode(w);
        smash.encode(w);
        combo.encode(w);
    }
}

impl Decode for Hook {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Hook {
            program: Decode::decode(r)?,
            path: decode_vec(r, max_path())?,
            left: Decode::decode(r)?,
            owner: Decode::decode(r)?,
            aim: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
            breaks_charge: Decode::decode(r)?,
            smash: Decode::decode(r)?,
            combo: Decode::decode(r)?,
        })
    }
}

impl Encode for OrbState {
    fn encode(&self, w: &mut Writer) {
        let OrbState {
            at,
            vx,
            vy,
            dir,
            style,
            owner,
            damage,
            damage_type,
            breaks_charge,
            smash,
            combo,
            hook,
            breakable,
            life,
            stop,
            link,
            program,
        } = self;
        at.encode(w);
        vx.encode(w);
        vy.encode(w);
        dir.encode(w);
        style.encode(w);
        owner.encode(w);
        damage.encode(w);
        damage_type.encode(w);
        breaks_charge.encode(w);
        smash.encode(w);
        combo.encode(w);
        hook.encode(w);
        breakable.encode(w);
        life.encode(w);
        stop.encode(w);
        link.encode(w);
        program.encode(w);
    }
}

impl Decode for OrbState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(OrbState {
            at: Decode::decode(r)?,
            vx: Decode::decode(r)?,
            vy: Decode::decode(r)?,
            dir: Decode::decode(r)?,
            style: Decode::decode(r)?,
            owner: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
            breaks_charge: Decode::decode(r)?,
            smash: Decode::decode(r)?,
            combo: Decode::decode(r)?,
            hook: Decode::decode(r)?,
            breakable: Decode::decode(r)?,
            life: Decode::decode(r)?,
            stop: Decode::decode(r)?,
            link: Decode::decode(r)?,
            program: Decode::decode(r)?,
        })
    }
}

impl Encode for ZoneState {
    fn encode(&self, w: &mut Writer) {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            wind_ticks,
            active_ticks,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
            program,
        } = self;
        owner.encode(w);
        friendly.encode(w);
        shape.encode(w);
        at.encode(w);
        dir.encode(w);
        wind_left.encode(w);
        active_left.encode(w);
        wind_ticks.encode(w);
        active_ticks.encode(w);
        damage.encode(w);
        damage_type.encode(w);
        breaks_charge.encode(w);
        warn_only.encode(w);
        hits.encode(w);
        program.encode(w);
    }
}

impl Decode for ZoneState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ZoneState {
            owner: Decode::decode(r)?,
            friendly: Decode::decode(r)?,
            shape: Decode::decode(r)?,
            at: Decode::decode(r)?,
            dir: Decode::decode(r)?,
            wind_left: Decode::decode(r)?,
            active_left: Decode::decode(r)?,
            wind_ticks: Decode::decode(r)?,
            active_ticks: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
            breaks_charge: Decode::decode(r)?,
            warn_only: Decode::decode(r)?,
            hits: decode_vec(r, max_hits())?,
            program: Decode::decode(r)?,
        })
    }
}

// ---- Agents, the boss and statuses ----

impl Encode for MoveOrder {
    fn encode(&self, w: &mut Writer) {
        let MoveOrder { to, ticks_left } = self;
        to.encode(w);
        ticks_left.encode(w);
    }
}

impl Decode for MoveOrder {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(MoveOrder {
            to: Decode::decode(r)?,
            ticks_left: Decode::decode(r)?,
        })
    }
}

impl Encode for RuleState {
    fn encode(&self, w: &mut Writer) {
        let RuleState {
            cooldown_left,
            charges_left,
            fired,
        } = self;
        cooldown_left.encode(w);
        charges_left.encode(w);
        fired.encode(w);
    }
}

impl Decode for RuleState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(RuleState {
            cooldown_left: Decode::decode(r)?,
            charges_left: Decode::decode(r)?,
            fired: Decode::decode(r)?,
        })
    }
}

impl Encode for StatusKind {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            StatusKind::AtkUp => 0,
            StatusKind::Vulnerable => 1,
            StatusKind::Slow => 2,
            StatusKind::Bleed => 3,
            StatusKind::Poison => 4,
            StatusKind::Burn => 5,
            StatusKind::Stun => 6,
        });
    }
}

impl Decode for StatusKind {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(match r.u8()? {
            0 => StatusKind::AtkUp,
            1 => StatusKind::Vulnerable,
            2 => StatusKind::Slow,
            3 => StatusKind::Bleed,
            4 => StatusKind::Poison,
            5 => StatusKind::Burn,
            6 => StatusKind::Stun,
            _ => return Err(DecodeError::Invalid("status kind tag")),
        })
    }
}

impl Encode for DotState {
    fn encode(&self, w: &mut Writer) {
        let DotState {
            damage,
            damage_type,
            atk_up_pct,
            type_bonus_pct,
            interval,
            next_in,
        } = self;
        damage.encode(w);
        damage_type.encode(w);
        atk_up_pct.encode(w);
        type_bonus_pct.encode(w);
        interval.encode(w);
        next_in.encode(w);
    }
}

impl Decode for DotState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(DotState {
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
            atk_up_pct: Decode::decode(r)?,
            type_bonus_pct: Decode::decode(r)?,
            interval: Decode::decode(r)?,
            next_in: Decode::decode(r)?,
        })
    }
}

impl Encode for StatusState {
    fn encode(&self, w: &mut Writer) {
        let StatusState {
            kind,
            source,
            remaining,
            total,
            stacks,
            value_pct,
            dot,
        } = self;
        kind.encode(w);
        source.encode(w);
        remaining.encode(w);
        total.encode(w);
        stacks.encode(w);
        value_pct.encode(w);
        dot.encode(w);
    }
}

impl Decode for StatusState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(StatusState {
            kind: Decode::decode(r)?,
            source: Decode::decode(r)?,
            remaining: Decode::decode(r)?,
            total: Decode::decode(r)?,
            stacks: Decode::decode(r)?,
            value_pct: Decode::decode(r)?,
            dot: Decode::decode(r)?,
        })
    }
}

impl Encode for AgentSource {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            AgentSource::Companion => 0,
            AgentSource::Wave => 1,
            AgentSource::Summoned => 2,
            AgentSource::BossRule => 3,
            AgentSource::Guardian => 4,
            AgentSource::Escort => 5,
        });
    }
}

impl Decode for AgentSource {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(match r.u8()? {
            0 => AgentSource::Companion,
            1 => AgentSource::Wave,
            2 => AgentSource::Summoned,
            3 => AgentSource::BossRule,
            4 => AgentSource::Guardian,
            5 => AgentSource::Escort,
            _ => return Err(DecodeError::Invalid("agent source tag")),
        })
    }
}

impl Encode for AgentKey {
    fn encode(&self, w: &mut Writer) {
        let AgentKey {
            source,
            index,
            rule,
        } = self;
        source.encode(w);
        index.encode(w);
        rule.encode(w);
    }
}

impl Decode for AgentKey {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(AgentKey {
            source: Decode::decode(r)?,
            index: Decode::decode(r)?,
            rule: Decode::decode(r)?,
        })
    }
}

impl Encode for ShrineState {
    fn encode(&self, w: &mut Writer) {
        let ShrineState { fight, guardians } = self;
        fight.encode(w);
        guardians.encode(w);
    }
}

impl Decode for ShrineState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ShrineState {
            fight: Decode::decode(r)?,
            guardians: decode_vec(r, max_guardians())?,
        })
    }
}

impl Encode for LeaveState {
    fn encode(&self, w: &mut Writer) {
        let LeaveState { disturbed, go } = self;
        disturbed.encode(w);
        go.encode(w);
    }
}

impl Decode for LeaveState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(LeaveState {
            disturbed: Decode::decode(r)?,
            go: Decode::decode(r)?,
        })
    }
}

impl Encode for EliteState {
    fn encode(&self, w: &mut Writer) {
        let EliteState {
            home,
            called,
            boss_call,
            disguised,
        } = self;
        home.encode(w);
        called.encode(w);
        boss_call.encode(w);
        disguised.encode(w);
    }
}

impl Decode for EliteState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(EliteState {
            home: Decode::decode(r)?,
            called: Decode::decode(r)?,
            boss_call: Decode::decode(r)?,
            disguised: Decode::decode(r)?,
        })
    }
}

impl Encode for AgentState {
    fn encode(&self, w: &mut Writer) {
        let AgentState {
            id,
            def,
            kind,
            key,
            at,
            hp,
            max_hp,
            defence,
            age,
            rules,
            statuses,
            aggro,
            order,
            orbit_angle,
            crowded,
            burst,
            stun_immune,
            elite,
            leave,
            shrine,
        } = self;
        id.encode(w);
        def.encode(w);
        kind.encode(w);
        key.encode(w);
        at.encode(w);
        hp.encode(w);
        max_hp.encode(w);
        defence.encode(w);
        age.encode(w);
        rules.encode(w);
        statuses.encode(w);
        aggro.encode(w);
        order.encode(w);
        orbit_angle.encode(w);
        crowded.encode(w);
        burst.encode(w);
        stun_immune.encode(w);
        elite.encode(w);
        leave.encode(w);
        shrine.encode(w);
    }
}

impl Decode for AgentState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(AgentState {
            id: Decode::decode(r)?,
            def: Decode::decode(r)?,
            kind: Decode::decode(r)?,
            key: Decode::decode(r)?,
            at: Decode::decode(r)?,
            hp: Decode::decode(r)?,
            max_hp: Decode::decode(r)?,
            defence: Decode::decode(r)?,
            age: Decode::decode(r)?,
            rules: Decode::decode(r)?,
            statuses: Decode::decode(r)?,
            aggro: Decode::decode(r)?,
            order: Decode::decode(r)?,
            orbit_angle: Decode::decode(r)?,
            crowded: Decode::decode(r)?,
            burst: Decode::decode(r)?,
            stun_immune: Decode::decode(r)?,
            elite: Decode::decode(r)?,
            leave: Decode::decode(r)?,
            shrine: Decode::decode(r)?,
        })
    }
}

impl Encode for PartState {
    fn encode(&self, w: &mut Writer) {
        let PartState {
            id,
            name,
            kind,
            hp,
            max_hp,
            defence,
            offset,
            radius,
            statuses,
        } = self;
        id.encode(w);
        name.encode(w);
        kind.encode(w);
        hp.encode(w);
        max_hp.encode(w);
        defence.encode(w);
        offset.encode(w);
        radius.encode(w);
        statuses.encode(w);
    }
}

impl Decode for PartState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(PartState {
            id: Decode::decode(r)?,
            name: Decode::decode(r)?,
            kind: Decode::decode(r)?,
            hp: Decode::decode(r)?,
            max_hp: Decode::decode(r)?,
            defence: Decode::decode(r)?,
            offset: Decode::decode(r)?,
            radius: Decode::decode(r)?,
            statuses: Decode::decode(r)?,
        })
    }
}

impl Encode for BossState {
    fn encode(&self, w: &mut Writer) {
        let BossState {
            id,
            boss,
            origin,
            no_call,
            kind,
            at,
            hp,
            max_hp,
            defence,
            statuses,
            phase,
            phase_age,
            invulnerable,
            step,
            wait_left,
            order,
            rules,
            parts,
            elites_called,
            age,
        } = self;
        id.encode(w);
        boss.encode(w);
        origin.encode(w);
        no_call.encode(w);
        kind.encode(w);
        at.encode(w);
        hp.encode(w);
        max_hp.encode(w);
        defence.encode(w);
        statuses.encode(w);
        phase.encode(w);
        phase_age.encode(w);
        invulnerable.encode(w);
        step.encode(w);
        wait_left.encode(w);
        order.encode(w);
        rules.encode(w);
        parts.encode(w);
        elites_called.encode(w);
        age.encode(w);
    }
}

impl Decode for BossState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(BossState {
            id: Decode::decode(r)?,
            boss: Decode::decode(r)?,
            origin: Decode::decode(r)?,
            no_call: Decode::decode(r)?,
            kind: Decode::decode(r)?,
            at: Decode::decode(r)?,
            hp: Decode::decode(r)?,
            max_hp: Decode::decode(r)?,
            defence: Decode::decode(r)?,
            statuses: Decode::decode(r)?,
            phase: Decode::decode(r)?,
            phase_age: Decode::decode(r)?,
            invulnerable: Decode::decode(r)?,
            step: Decode::decode(r)?,
            wait_left: Decode::decode(r)?,
            order: Decode::decode(r)?,
            rules: Decode::decode(r)?,
            parts: decode_vec(r, max_parts())?,
            elites_called: Decode::decode(r)?,
            age: Decode::decode(r)?,
        })
    }
}

// ---- Echoes ----

impl Encode for Echo {
    fn encode(&self, w: &mut Writer) {
        let Echo {
            source,
            target,
            damage,
            damage_type,
        } = self;
        source.encode(w);
        target.encode(w);
        damage.encode(w);
        damage_type.encode(w);
    }
}

impl Decode for Echo {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Echo {
            source: Decode::decode(r)?,
            target: Decode::decode(r)?,
            damage: Decode::decode(r)?,
            damage_type: Decode::decode(r)?,
        })
    }
}

// ---- The generators ----

impl Encode for Streams {
    /// Five `u64`, each generator's state, in the order the hash writes them.
    fn encode(&self, w: &mut Writer) {
        let Streams {
            spawn,
            attack,
            loot,
            offer,
            horde,
        } = self;
        spawn.encode(w);
        attack.encode(w);
        loot.encode(w);
        offer.encode(w);
        horde.encode(w);
    }
}

impl Decode for Streams {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Streams {
            spawn: Decode::decode(r)?,
            attack: Decode::decode(r)?,
            loot: Decode::decode(r)?,
            offer: Decode::decode(r)?,
            horde: Decode::decode(r)?,
        })
    }
}

// ---- Enums of the state ----

/// A fieldless enum: a literal tag per variant, the numbers its `StableHash` writes, and no
/// wildcard arm, so a variant added without a number does not compile.
macro_rules! enum_codec {
    ($($ty:ty, $what:literal: { $($variant:path => $tag:literal),* $(,)? });* $(;)?) => {$(
        impl Encode for $ty {
            fn encode(&self, w: &mut Writer) {
                w.u8(match self {
                    $($variant => $tag,)*
                });
            }
        }

        impl Decode for $ty {
            fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
                match r.u8()? {
                    $($tag => Ok($variant),)*
                    _ => Err(DecodeError::Invalid($what)),
                }
            }
        }
    )*};
}

enum_codec! {
    Outcome, "outcome tag": {
        Outcome::Running => 0,
        Outcome::Cleared => 1,
        Outcome::Failed => 2,
    };
    Rarity, "rarity tag": {
        Rarity::Common => 0,
        Rarity::Uncommon => 1,
        Rarity::Rare => 2,
        Rarity::Epic => 3,
        Rarity::Legendary => 4,
        Rarity::Supernova => 5,
    };
    OfferKind, "offer kind tag": {
        OfferKind::Star => 0,
        OfferKind::Boss => 1,
    };
    ExpertiseOfferKind, "expertise offer kind tag": {
        ExpertiseOfferKind::Common => 0,
        ExpertiseOfferKind::Style => 1,
        ExpertiseOfferKind::Upgrade => 2,
        ExpertiseOfferKind::Capstone => 3,
    };
    Rune, "rune tag": {
        Rune::Explosion => 0,
        Rune::Thunder => 1,
        Rune::Boomerang => 2,
        Rune::HomingMissiles => 3,
        Rune::Bleed => 4,
        Rune::Poison => 5,
        Rune::Burn => 6,
    };
    RangedPool, "ranged pool tag": {
        RangedPool::Zone => 0,
        RangedPool::Line => 1,
    };
}

// ---- The caps of the lists below ----

/// A cap, from the constant that bounds the list in play.
fn cap_u32(n: usize) -> u32 {
    u32::try_from(n).expect("a small cap")
}

/// The most places a list of `u8` places into a content file holds.
const MAX_PLACES: u32 = 256;

/// The cap of a list whose length is the content's (waves, bosses): `decode_state` checks it
/// against the fresh engine's, and the bytes left bound the length before that.
const MAX_CONTENT_LEN: u32 = 1 << 16;

// ---- The player ----

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
    }
}

impl Decode for PlayerState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(PlayerState {
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

// ---- Progression: levels, stars, offers, healing, expertise and the horde ----

impl Encode for Progress {
    fn encode(&self, w: &mut Writer) {
        let Progress {
            level,
            xp,
            // not saved: the run stats' rules are content: `decode_state` copies them from the fresh engine
            stats: _,
            priority,
            pick_stats,
            offers_due,
            offer_open,
            spent,
            totals,
        } = self;
        level.encode(w);
        xp.encode(w);
        priority.encode(w);
        pick_stats.encode(w);
        offers_due.encode(w);
        offer_open.encode(w);
        spent.encode(w);
        totals.encode(w);
    }
}

impl Decode for Progress {
    /// `stats` is left empty: `decode_state` copies it from the fresh engine.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Progress {
            level: Decode::decode(r)?,
            xp: Decode::decode(r)?,
            stats: Vec::new(),
            priority: decode_vec(r, MAX_PLACES)?,
            pick_stats: Decode::decode(r)?,
            offers_due: Decode::decode(r)?,
            offer_open: Decode::decode(r)?,
            spent: decode_vec(r, MAX_PLACES)?,
            totals: Decode::decode(r)?,
        })
    }
}

impl Encode for StarField {
    fn encode(&self, w: &mut Writer) {
        let StarField {
            misses,
            on_map,
            due,
            offer,
            rerolls_left,
            banishes_left,
            extra_candidates,
            rarity,
            rune_pool,
            pool,
            taken,
            banished,
            set_counts,
        } = self;
        misses.encode(w);
        on_map.encode(w);
        due.encode(w);
        offer.encode(w);
        rerolls_left.encode(w);
        banishes_left.encode(w);
        extra_candidates.encode(w);
        rarity.encode(w);
        rune_pool.encode(w);
        pool.encode(w);
        taken.encode(w);
        banished.encode(w);
        set_counts.encode(w);
    }
}

impl Decode for StarField {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(StarField {
            misses: Decode::decode(r)?,
            on_map: decode_vec(r, cap_u32(usize::from(crate::limits::MAX_STARS_ON_MAP)))?,
            due: decode_vec(r, cap_u32(usize::from(crate::limits::MAX_STARS_ON_MAP) + 1))?,
            offer: Decode::decode(r)?,
            rerolls_left: Decode::decode(r)?,
            banishes_left: Decode::decode(r)?,
            extra_candidates: Decode::decode(r)?,
            rarity: Decode::decode(r)?,
            rune_pool: Decode::decode(r)?,
            pool: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
            taken: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
            banished: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
            set_counts: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
        })
    }
}

impl Encode for StarState {
    fn encode(&self, w: &mut Writer) {
        let StarState { id, at, rarity } = self;
        id.encode(w);
        at.encode(w);
        rarity.encode(w);
    }
}

impl Decode for StarState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(StarState {
            id: Decode::decode(r)?,
            at: Decode::decode(r)?,
            rarity: Decode::decode(r)?,
        })
    }
}

impl Encode for Due {
    fn encode(&self, w: &mut Writer) {
        let Due { kind, rarity } = self;
        kind.encode(w);
        rarity.encode(w);
    }
}

impl Decode for Due {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Due {
            kind: Decode::decode(r)?,
            rarity: Decode::decode(r)?,
        })
    }
}

impl Encode for OfferState {
    fn encode(&self, w: &mut Writer) {
        let OfferState {
            kind,
            rarity,
            candidates,
        } = self;
        kind.encode(w);
        rarity.encode(w);
        candidates.encode(w);
    }
}

impl Decode for OfferState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(OfferState {
            kind: Decode::decode(r)?,
            rarity: Decode::decode(r)?,
            candidates: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
        })
    }
}

impl Encode for HealPickup {
    fn encode(&self, w: &mut Writer) {
        let HealPickup { id, at } = self;
        id.encode(w);
        at.encode(w);
    }
}

impl Decode for HealPickup {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(HealPickup {
            id: Decode::decode(r)?,
            at: Decode::decode(r)?,
        })
    }
}

impl Encode for ExpertiseGauge {
    fn encode(&self, w: &mut Writer) {
        let ExpertiseGauge {
            level,
            points,
            next,
            offered,
            offer,
            taken,
            range_pct,
            style,
            path,
        } = self;
        level.encode(w);
        points.encode(w);
        next.encode(w);
        offered.encode(w);
        offer.encode(w);
        taken.encode(w);
        range_pct.encode(w);
        style.encode(w);
        path.encode(w);
    }
}

impl Decode for ExpertiseGauge {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ExpertiseGauge {
            level: Decode::decode(r)?,
            points: Decode::decode(r)?,
            next: Decode::decode(r)?,
            offered: Decode::decode(r)?,
            offer: Decode::decode(r)?,
            taken: decode_vec(r, MAX_PLACES)?,
            range_pct: Decode::decode(r)?,
            style: Decode::decode(r)?,
            path: decode_vec(r, MAX_PLACES)?,
        })
    }
}

impl Encode for ExpertiseOffer {
    fn encode(&self, w: &mut Writer) {
        let ExpertiseOffer {
            level,
            kind,
            candidates,
        } = self;
        level.encode(w);
        kind.encode(w);
        candidates.encode(w);
    }
}

impl Decode for ExpertiseOffer {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ExpertiseOffer {
            level: Decode::decode(r)?,
            kind: Decode::decode(r)?,
            candidates: decode_vec(r, MAX_PLACES)?,
        })
    }
}

impl Encode for Horde {
    fn encode(&self, w: &mut Writer) {
        let Horde {
            // not saved: the mode's boss times are content: `decode_state` copies them from the fresh engine
            boss_ticks: _,
            // not saved: the mode's no-call times are content: `decode_state` copies them from the fresh engine
            no_call_ticks: _,
            // not saved: the mid-bosses' hit points are content: `decode_state` copies them from the fresh engine
            mid_boss_hp_pct: _,
            spawned,
            used,
            held,
        } = self;
        spawned.encode(w);
        used.encode(w);
        held.encode(w);
    }
}

impl Decode for Horde {
    /// The times are left empty: `decode_state` copies them from the fresh engine.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Horde {
            boss_ticks: Vec::new(),
            no_call_ticks: Vec::new(),
            mid_boss_hp_pct: Vec::new(),
            spawned: Decode::decode(r)?,
            used: decode_vec(r, MAX_CONTENT_LEN)?,
            held: decode_vec(r, MAX_CONTENT_LEN)?,
        })
    }
}

// ---- Text ----

/// The most bytes of a name the ledger and the tallies hold (a kill key, a star's name).
const MAX_TEXT_BYTES: u32 = 256;

impl Encode for str {
    fn encode(&self, w: &mut Writer) {
        w.len(self.len());
        w.bytes(self.as_bytes());
    }
}

impl Encode for String {
    fn encode(&self, w: &mut Writer) {
        self.as_str().encode(w);
    }
}

impl Encode for std::sync::Arc<str> {
    fn encode(&self, w: &mut Writer) {
        (**self).encode(w);
    }
}

/// The text of at most `MAX_TEXT_BYTES` bytes.
fn decode_text<'a>(r: &mut Reader<'a>) -> Result<&'a str, DecodeError> {
    let n = r.list_len(MAX_TEXT_BYTES)?;
    std::str::from_utf8(r.bytes(n)?).map_err(|_| DecodeError::Invalid("utf8"))
}

impl Decode for String {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        decode_text(r).map(str::to_owned)
    }
}

impl Decode for std::sync::Arc<str> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        decode_text(r).map(std::sync::Arc::from)
    }
}

// ---- The state ----

impl Encode for State {
    fn encode(&self, w: &mut Writer) {
        let State {
            // not saved: buffers reused from tick to tick, never state.
            scratch: _,
            // not saved: the run's kinds are content; `decode_state` takes them from the fresh engine.
            kinds: _,
            tick,
            rng,
            outcome,
            player,
            boss,
            companions,
            enemies,
            tasks,
            runs,
            // not saved: derived from `runs` and the run's attacks; `decode_state` rebuilds it.
            move_index: _,
            ranged_cooling,
            orbs,
            zones,
            spawned,
            next_id,
            next_orb_group,
            progress,
            stars,
            heal_pickups,
            expertise,
            // not saved: content; `decode_state` takes it from the fresh engine.
            match_ups: _,
            // not saved: content, as the match-ups are.
            runes: _,
            // not saved: content, as the match-ups are.
            stun: _,
            max_attack_speed_pct,
            enemy_attack_damage,
            max_rune_haste_pct,
            horde,
            stardust,
            regen,
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
        } = self;
        tick.encode(w);
        rng.encode(w);
        outcome.encode(w);
        player.encode(w);
        boss.encode(w);
        companions.encode(w);
        enemies.encode(w);
        tasks.encode(w);
        runs.encode(w);
        ranged_cooling.encode(w);
        orbs.encode(w);
        zones.encode(w);
        spawned.encode(w);
        next_id.encode(w);
        next_orb_group.encode(w);
        progress.encode(w);
        stars.encode(w);
        heal_pickups.encode(w);
        expertise.encode(w);
        max_attack_speed_pct.encode(w);
        enemy_attack_damage.encode(w);
        max_rune_haste_pct.encode(w);
        horde.encode(w);
        stardust.encode(w);
        regen.encode(w);
        hp_pct.encode(w);
        damage_pct.encode(w);
        crit_seed.encode(w);
        echoes.encode(w);
    }
}

impl State {
    /// The state in `r`, with the fields it does not save (the content's tables and buffers)
    /// taken from `fresh`, the state of an engine started from the same setup and content. The
    /// lists are only as long as the format allows: `decode_state` checks them against `fresh`,
    /// and fills the index and the content-sized fields of `progress` and `horde` in.
    pub(crate) fn decode_over(r: &mut Reader<'_>, fresh: &State) -> Result<State, DecodeError> {
        let companions_cap = cap_u32(crate::validate::limits::MAX_COMPANIONS);
        let enemies_cap = u32::from(crate::limits::MAX_ENEMY_CAP);
        Ok(State {
            scratch: Default::default(),
            kinds: fresh.kinds.clone(),
            tick: Decode::decode(r)?,
            rng: Decode::decode(r)?,
            outcome: Decode::decode(r)?,
            player: Decode::decode(r)?,
            boss: Decode::decode(r)?,
            companions: decode_vec(r, companions_cap)?,
            enemies: decode_vec(r, enemies_cap)?,
            tasks: decode_vec(r, crate::limits::MAX_ATTACK_TASKS)?,
            runs: decode_vec(r, enemies_cap + companions_cap + 1)?,
            move_index: Default::default(),
            ranged_cooling: decode_vec(r, crate::limits::MAX_RANGED_ATTACKERS)?,
            orbs: decode_vec(r, crate::limits::MAX_ORBS_ALIVE)?,
            zones: decode_vec(r, crate::limits::MAX_ZONES_ALIVE)?,
            spawned: decode_vec(r, MAX_CONTENT_LEN)?,
            next_id: Decode::decode(r)?,
            next_orb_group: Decode::decode(r)?,
            progress: Decode::decode(r)?,
            stars: Decode::decode(r)?,
            heal_pickups: match r.u8()? {
                0 => None,
                1 => Some(decode_vec(
                    r,
                    u32::from(crate::validate::limits::MAX_HEAL_PICKUPS),
                )?),
                _ => return Err(DecodeError::Invalid("option tag")),
            },
            expertise: Decode::decode(r)?,
            match_ups: fresh.match_ups,
            runes: fresh.runes.clone(),
            stun: fresh.stun,
            max_attack_speed_pct: Decode::decode(r)?,
            enemy_attack_damage: Decode::decode(r)?,
            max_rune_haste_pct: Decode::decode(r)?,
            horde: Decode::decode(r)?,
            stardust: Decode::decode(r)?,
            regen: Decode::decode(r)?,
            hp_pct: Decode::decode(r)?,
            damage_pct: Decode::decode(r)?,
            crit_seed: Decode::decode(r)?,
            echoes: decode_vec(r, cap_u32(crate::limits::MAX_ECHOES_PER_TICK))?,
        })
    }
}

#[cfg(test)]
mod enum_tests {
    use super::*;
    use crate::savepoint::testing::{from_bytes, to_bytes};

    /// Every variant, in order, is its number; the number after the last is refused.
    fn tags<T: Encode + Decode + PartialEq + std::fmt::Debug>(all: &[T]) {
        for (i, v) in all.iter().enumerate() {
            assert_eq!(to_bytes(v), [u8::try_from(i).unwrap()], "{v:?}");
            assert_eq!(from_bytes::<T>(&[u8::try_from(i).unwrap()]).as_ref(), Ok(v));
        }
        let past = u8::try_from(all.len()).unwrap();
        assert!(matches!(
            from_bytes::<T>(&[past]),
            Err(DecodeError::Invalid(_))
        ));
        assert_eq!(from_bytes::<T>(&[]), Err(DecodeError::Truncated));
    }

    #[test]
    fn the_enums_of_the_state_are_their_hash_numbers() {
        tags(&[Outcome::Running, Outcome::Cleared, Outcome::Failed]);
        tags(&[
            Rarity::Common,
            Rarity::Uncommon,
            Rarity::Rare,
            Rarity::Epic,
            Rarity::Legendary,
            Rarity::Supernova,
        ]);
        tags(&[OfferKind::Star, OfferKind::Boss]);
        tags(&[
            ExpertiseOfferKind::Common,
            ExpertiseOfferKind::Style,
            ExpertiseOfferKind::Upgrade,
            ExpertiseOfferKind::Capstone,
        ]);
        tags(&[
            Rune::Explosion,
            Rune::Thunder,
            Rune::Boomerang,
            Rune::HomingMissiles,
            Rune::Bleed,
            Rune::Poison,
            Rune::Burn,
        ]);
        tags(&[RangedPool::Zone, RangedPool::Line]);
    }
}
