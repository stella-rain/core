//! The savepoint encoding of the agents, the shrines and the boss.

use crate::savepoint::codec::{
    Decode, DecodeError, Encode, Reader, Writer, decode_vec, max_guardians, max_parts,
};
use crate::sim::state::*;

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
