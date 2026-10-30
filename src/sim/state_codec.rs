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
