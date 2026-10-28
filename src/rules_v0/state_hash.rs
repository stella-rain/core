//! The state hash of every `State` type (ADR-SIM-3 point 2): each impl destructures its type
//! without `..`, so a field added without a decision does not compile. A field that is left out
//! is `field: _` with `// not hashed: <why>`; `engine::hash_coverage_tests` lists them again and
//! checks both halves.
//!
//! The fields are written in declaration order. A list writes its length first, an `Option` and
//! an enum a tag byte (`hash_enum!`, a literal in each impl), so the encoding is prefix-free.

use crate::attack::{DamageType, ZoneShape};
use crate::hash::{StableHash, StateHasher, hash_enum};

use super::state::*;

// ---- Attacks, orbs and zones ----

impl StableHash for Dir {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Dir { x, y } = self;
        x.stable_hash(h);
        y.stable_hash(h);
    }
}

impl StableHash for PathStep {
    fn stable_hash(&self, h: &mut StateHasher) {
        let PathStep { node, then } = self;
        node.stable_hash(h);
        then.stable_hash(h);
    }
}

impl StableHash for Frame {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Frame {
            idx,
            loop_i,
            loop_n,
            param,
            offset,
        } = self;
        idx.stable_hash(h);
        loop_i.stable_hash(h);
        loop_n.stable_hash(h);
        param.stable_hash(h);
        offset.stable_hash(h);
    }
}

impl StableHash for Aim {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Aim::Entity(id) => {
                hash_enum!(h, 0);
                id.stable_hash(h);
            }
            Aim::Fixed(dir) => {
                hash_enum!(h, 1);
                dir.stable_hash(h);
            }
        }
    }
}

impl StableHash for Origin {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Origin::Owner => hash_enum!(h, 0),
            Origin::At(at) => {
                hash_enum!(h, 1);
                at.stable_hash(h);
            }
        }
    }
}

impl StableHash for Pull {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Pull { distance, ticks } = self;
        distance.stable_hash(h);
        ticks.stable_hash(h);
    }
}

impl StableHash for Smash {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Smash::Pull(pull) => {
                hash_enum!(h, 0);
                pull.stable_hash(h);
            }
            Smash::Knockback(pull) => {
                hash_enum!(h, 1);
                pull.stable_hash(h);
            }
            Smash::Bleed => hash_enum!(h, 2),
            Smash::Stun { ticks } => {
                hash_enum!(h, 3);
                ticks.stable_hash(h);
            }
        }
    }
}

impl StableHash for DamageType {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            DamageType::Slash => hash_enum!(h, 0),
            DamageType::Pierce => hash_enum!(h, 1),
            DamageType::Strike => hash_enum!(h, 2),
            DamageType::Pyro => hash_enum!(h, 3),
            DamageType::Hydro => hash_enum!(h, 4),
            DamageType::Electro => hash_enum!(h, 5),
            DamageType::Cryo => hash_enum!(h, 6),
            DamageType::Anemo => hash_enum!(h, 7),
            DamageType::Geo => hash_enum!(h, 8),
            DamageType::Dendro => hash_enum!(h, 9),
        }
    }
}

impl StableHash for ZoneShape {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            ZoneShape::Circle { radius } => {
                hash_enum!(h, 0);
                radius.stable_hash(h);
            }
            ZoneShape::Arc { radius, half_angle } => {
                hash_enum!(h, 1);
                radius.stable_hash(h);
                half_angle.stable_hash(h);
            }
            ZoneShape::Line { length, width } => {
                hash_enum!(h, 2);
                length.stable_hash(h);
                width.stable_hash(h);
            }
        }
    }
}

impl StableHash for AttackTask {
    fn stable_hash(&self, h: &mut StateHasher) {
        let AttackTask {
            owner,
            // not hashed: a preset and its inline copy are the same task and must hash alike
            // (`a_preset_and_its_inline_copy_reach_the_same_hashes`); where the task has got to
            // is hashed, and the program is content.
            program: _,
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
        owner.stable_hash(h);
        base.stable_hash(h);
        origin.stable_hash(h);
        aim.stable_hash(h);
        friendly.stable_hash(h);
        damage.stable_hash(h);
        damage_type.stable_hash(h);
        breaks_charge.stable_hash(h);
        smash.stable_hash(h);
        combo.stable_hash(h);
        age.stable_hash(h);
        wait_left.stable_hash(h);
        frames.stable_hash(h);
        parent_dir.stable_hash(h);
        last_dir.stable_hash(h);
        spot.stable_hash(h);
    }
}

impl StableHash for AttackRun {
    fn stable_hash(&self, h: &mut StateHasher) {
        let AttackRun {
            owner,
            slot,
            // not hashed: an index into the run's compiled attacks, fixed by the content hash;
            // the phase lengths are a function of it and `speed_pct`.
            attack: _,
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
        owner.stable_hash(h);
        slot.stable_hash(h);
        speed_pct.stable_hash(h);
        width_pct.stable_hash(h);
        phase.stable_hash(h);
        tick.stable_hash(h);
        facing.stable_hash(h);
        aim.stable_hash(h);
        friendly.stable_hash(h);
        ranged.stable_hash(h);
        base.stable_hash(h);
        atk_up_pct.stable_hash(h);
        damage_pct.stable_hash(h);
        type_bonus_pct.stable_hash(h);
        age.stable_hash(h);
        hits.stable_hash(h);
        smash.stable_hash(h);
        at.stable_hash(h);
        throw.stable_hash(h);
        homing.stable_hash(h);
        spot.stable_hash(h);
    }
}

impl StableHash for OrbLink {
    fn stable_hash(&self, h: &mut StateHasher) {
        let OrbLink {
            group,
            slot,
            of,
            width,
            damage,
            rehit_ticks,
            last_hit,
        } = self;
        group.stable_hash(h);
        slot.stable_hash(h);
        of.stable_hash(h);
        width.stable_hash(h);
        damage.stable_hash(h);
        rehit_ticks.stable_hash(h);
        last_hit.stable_hash(h);
    }
}

impl StableHash for Hook {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Hook {
            // not hashed: an index into the run's programs, as `AttackTask::program` is.
            program: _,
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
        path.stable_hash(h);
        left.stable_hash(h);
        owner.stable_hash(h);
        aim.stable_hash(h);
        damage.stable_hash(h);
        damage_type.stable_hash(h);
        breaks_charge.stable_hash(h);
        smash.stable_hash(h);
        combo.stable_hash(h);
    }
}

impl StableHash for OrbState {
    fn stable_hash(&self, h: &mut StateHasher) {
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
        } = self;
        at.stable_hash(h);
        vx.stable_hash(h);
        vy.stable_hash(h);
        dir.stable_hash(h);
        style.stable_hash(h);
        owner.stable_hash(h);
        damage.stable_hash(h);
        damage_type.stable_hash(h);
        breaks_charge.stable_hash(h);
        smash.stable_hash(h);
        combo.stable_hash(h);
        hook.stable_hash(h);
        breakable.stable_hash(h);
        life.stable_hash(h);
        stop.stable_hash(h);
        link.stable_hash(h);
    }
}

impl StableHash for ZoneState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            // not hashed: the warning's and the hurting's length when the zone was placed, for
            // the snapshot only; `wind_left` and `active_left` carry the state.
            wind_ticks: _,
            active_ticks: _,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
        } = self;
        owner.stable_hash(h);
        friendly.stable_hash(h);
        shape.stable_hash(h);
        at.stable_hash(h);
        dir.stable_hash(h);
        wind_left.stable_hash(h);
        active_left.stable_hash(h);
        damage.stable_hash(h);
        damage_type.stable_hash(h);
        breaks_charge.stable_hash(h);
        warn_only.stable_hash(h);
        hits.stable_hash(h);
    }
}
