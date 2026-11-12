//! The savepoint encoding of the attack run and its steps.

use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec, max_hits};
use crate::sim::state::*;

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
            base: Decode::decode(r)?,
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
