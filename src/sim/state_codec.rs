//! The savepoint encoding of every `State` type, beside the hash in `state_hash.rs` and in the
//! same order: each impl destructures its type without `..`, so a field added without a decision
//! does not compile. Every field is saved, the content indices the hash skips included, because
//! they are what a restored run reads; the pack hash guards that they mean the same content.
//!
//! The layout is the fields in declaration order, an enum a `u8` tag (the number its
//! `StableHash` writes), an `Option` a `0`/`1` tag, a list a `u32` length (see `savepoint`).

use crate::savepoint::codec::{
    Decode, DecodeError, Encode, Reader, Writer, decode_vec, max_hits, max_path,
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
