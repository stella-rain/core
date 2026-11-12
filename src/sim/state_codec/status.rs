//! The savepoint encoding of the status types.

use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer};
use crate::sim::state::*;

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
