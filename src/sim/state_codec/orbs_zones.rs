//! The savepoint encoding of the aim, orb and zone types.

use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec, max_hits};
use crate::sim::state::*;

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
            path: Decode::decode(r)?,
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
