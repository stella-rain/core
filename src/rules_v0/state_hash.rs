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

// ---- Agents, the boss and statuses ----

impl StableHash for MoveOrder {
    fn stable_hash(&self, h: &mut StateHasher) {
        let MoveOrder { to, ticks_left } = self;
        to.stable_hash(h);
        ticks_left.stable_hash(h);
    }
}

impl StableHash for RuleState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let RuleState {
            cooldown_left,
            charges_left,
            fired,
        } = self;
        cooldown_left.stable_hash(h);
        charges_left.stable_hash(h);
        fired.stable_hash(h);
    }
}

impl StableHash for StatusKind {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            StatusKind::AtkUp => hash_enum!(h, 0),
            StatusKind::Vulnerable => hash_enum!(h, 1),
            StatusKind::Slow => hash_enum!(h, 2),
            StatusKind::Bleed => hash_enum!(h, 3),
            StatusKind::Poison => hash_enum!(h, 4),
            StatusKind::Burn => hash_enum!(h, 5),
            StatusKind::Stun => hash_enum!(h, 6),
        }
    }
}

impl StableHash for DotState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let DotState {
            damage,
            damage_type,
            atk_up_pct,
            type_bonus_pct,
            interval,
            next_in,
        } = self;
        damage.stable_hash(h);
        damage_type.stable_hash(h);
        atk_up_pct.stable_hash(h);
        type_bonus_pct.stable_hash(h);
        interval.stable_hash(h);
        next_in.stable_hash(h);
    }
}

impl StableHash for StatusState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let StatusState {
            kind,
            source,
            remaining,
            total,
            stacks,
            value_pct,
            dot,
        } = self;
        kind.stable_hash(h);
        source.stable_hash(h);
        remaining.stable_hash(h);
        total.stable_hash(h);
        stacks.stable_hash(h);
        value_pct.stable_hash(h);
        dot.stable_hash(h);
    }
}

impl StableHash for AgentSource {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            AgentSource::Companion => hash_enum!(h, 0),
            AgentSource::Wave => hash_enum!(h, 1),
            AgentSource::Summoned => hash_enum!(h, 2),
            AgentSource::BossRule => hash_enum!(h, 3),
            AgentSource::Guardian => hash_enum!(h, 4),
            AgentSource::Escort => hash_enum!(h, 5),
        }
    }
}

impl StableHash for AgentKey {
    fn stable_hash(&self, h: &mut StateHasher) {
        let AgentKey {
            source,
            index,
            rule,
        } = self;
        source.stable_hash(h);
        index.stable_hash(h);
        rule.stable_hash(h);
    }
}

impl StableHash for ShrineState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let ShrineState { fight, guardians } = self;
        fight.stable_hash(h);
        guardians.stable_hash(h);
    }
}

impl StableHash for LeaveState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let LeaveState { disturbed, go } = self;
        disturbed.stable_hash(h);
        go.stable_hash(h);
    }
}

impl StableHash for EliteState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let EliteState {
            home,
            called,
            // not hashed: what the boss's call does to the elite is its definition's.
            boss_call: _,
            disguised,
        } = self;
        home.stable_hash(h);
        called.stable_hash(h);
        disguised.stable_hash(h);
    }
}

impl StableHash for AgentState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let AgentState {
            id,
            // not hashed: an index into the run's definitions, fixed by the content hash.
            def: _,
            // not hashed: an index into the run's kinds, made at spawn from content.
            kind: _,
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
        id.stable_hash(h);
        key.stable_hash(h);
        at.stable_hash(h);
        hp.stable_hash(h);
        max_hp.stable_hash(h);
        defence.stable_hash(h);
        age.stable_hash(h);
        rules.stable_hash(h);
        statuses.stable_hash(h);
        aggro.stable_hash(h);
        order.stable_hash(h);
        orbit_angle.stable_hash(h);
        crowded.stable_hash(h);
        burst.stable_hash(h);
        stun_immune.stable_hash(h);
        elite.stable_hash(h);
        leave.stable_hash(h);
        shrine.stable_hash(h);
    }
}

impl StableHash for PartState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let PartState {
            id,
            // not hashed: the stage's ID for the part, content.
            name: _,
            // not hashed: an index into the run's kinds, made at spawn from content.
            kind: _,
            hp,
            max_hp,
            defence,
            offset,
            radius,
            statuses,
        } = self;
        id.stable_hash(h);
        hp.stable_hash(h);
        max_hp.stable_hash(h);
        defence.stable_hash(h);
        offset.stable_hash(h);
        radius.stable_hash(h);
        statuses.stable_hash(h);
    }
}

impl StableHash for BossState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let BossState {
            id,
            // not hashed: an index into the run's boss definitions, fixed by the content hash.
            boss: _,
            origin,
            no_call,
            // not hashed: an index into the run's kinds, made at spawn from content.
            kind: _,
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
        id.stable_hash(h);
        origin.stable_hash(h);
        no_call.stable_hash(h);
        at.stable_hash(h);
        hp.stable_hash(h);
        max_hp.stable_hash(h);
        defence.stable_hash(h);
        statuses.stable_hash(h);
        phase.stable_hash(h);
        phase_age.stable_hash(h);
        invulnerable.stable_hash(h);
        step.stable_hash(h);
        wait_left.stable_hash(h);
        order.stable_hash(h);
        rules.stable_hash(h);
        parts.stable_hash(h);
        elites_called.stable_hash(h);
        age.stable_hash(h);
    }
}
