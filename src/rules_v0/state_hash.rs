//! The state hash of every `State` type (ADR-SIM-3 point 2): each impl destructures its type
//! without `..`, so a field added without a decision does not compile. A field that is left out
//! is `field: _` with `// not hashed: <why>`; `engine::hash_coverage_tests` lists them again and
//! checks both halves.
//!
//! The fields are written in declaration order. A list writes its length first, an `Option` and
//! an enum a tag byte (`hash_enum!`, a literal in each impl), so the encoding is prefix-free.

use crate::attack::{DamageType, ZoneShape};
use crate::engine::Outcome;
use crate::hash::{StableHash, StateHasher, hash_enum};
use crate::rewards::Rarity;
use crate::rules_v0::RangedPool;
use crate::rune::Rune;
use crate::snapshot::{ExpertiseOfferKind, OfferKind};

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

// ---- The player, progression and the whole State ----

impl StableHash for ComboState {
    fn stable_hash(&self, h: &mut StateHasher) {
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
        path.stable_hash(h);
        depth.stable_hash(h);
        dash.stable_hash(h);
        buffered.stable_hash(h);
        running.stable_hash(h);
        window_left.stable_hash(h);
        dash_strike_left.stable_hash(h);
        charge.stable_hash(h);
        hold.stable_hash(h);
        released.stable_hash(h);
        hand.stable_hash(h);
        auto_path.stable_hash(h);
        auto_depth.stable_hash(h);
        levels.stable_hash(h);
        auto_levels.stable_hash(h);
        charge_auto.stable_hash(h);
    }
}

impl StableHash for SkillSlot {
    fn stable_hash(&self, h: &mut StateHasher) {
        let SkillSlot {
            index,
            level,
            cooldown_left,
        } = self;
        index.stable_hash(h);
        level.stable_hash(h);
        cooldown_left.stable_hash(h);
    }
}

impl StableHash for OrbitState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let OrbitState {
            reward,
            orbs,
            round,
            cooldown_left,
            angle,
        } = self;
        reward.stable_hash(h);
        orbs.stable_hash(h);
        round.stable_hash(h);
        cooldown_left.stable_hash(h);
        angle.stable_hash(h);
    }
}

impl StableHash for HeldRune {
    fn stable_hash(&self, h: &mut StateHasher) {
        let HeldRune {
            rune,
            cooldown_left,
        } = self;
        rune.stable_hash(h);
        cooldown_left.stable_hash(h);
    }
}

impl StableHash for PullState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let PullState {
            at,
            radius,
            strength,
            ticks_left,
        } = self;
        at.stable_hash(h);
        radius.stable_hash(h);
        strength.stable_hash(h);
        ticks_left.stable_hash(h);
    }
}

impl StableHash for ShieldState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let ShieldState { amount, ticks_left } = self;
        amount.stable_hash(h);
        ticks_left.stable_hash(h);
    }
}

impl StableHash for DashState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let DashState { dir, done, left } = self;
        dir.stable_hash(h);
        done.stable_hash(h);
        left.stable_hash(h);
    }
}

impl StableHash for PlayerState {
    fn stable_hash(&self, h: &mut StateHasher) {
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
        at.stable_hash(h);
        hp.stable_hash(h);
        max_hp.stable_hash(h);
        defence.stable_hash(h);
        invulnerable.stable_hash(h);
        facing.stable_hash(h);
        target.stable_hash(h);
        evade_charges.stable_hash(h);
        evade_recovering.stable_hash(h);
        held.stable_hash(h);
        combo.stable_hash(h);
        dash.stable_hash(h);
        skill.stable_hash(h);
        pull.stable_hash(h);
        shield.stable_hash(h);
        orbits.stable_hash(h);
        range_pct.stable_hash(h);
        rune_haste_pct.stable_hash(h);
        pickup_radius_pct.stable_hash(h);
        skill_cooldown_pct.stable_hash(h);
        kill_heal.stable_hash(h);
        kill_heal_every.stable_hash(h);
        kills_counted.stable_hash(h);
        crit_chance_pct.stable_hash(h);
        crit_damage_pct.stable_hash(h);
        echo_hit_pct.stable_hash(h);
        burst_pct.stable_hash(h);
        burst_radius.stable_hash(h);
        burst_depth.stable_hash(h);
        burst_depth_extra.stable_hash(h);
        type_bonus_pct.stable_hash(h);
        runes.stable_hash(h);
        aggro.stable_hash(h);
        statuses.stable_hash(h);
    }
}

impl StableHash for Progress {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Progress {
            level,
            xp,
            // not hashed: the run stats' rules, in the run stats' order: content, fixed for the run.
            stats: _,
            priority,
            pick_stats,
            offers_due,
            offer_open,
            spent,
            totals,
        } = self;
        level.stable_hash(h);
        xp.stable_hash(h);
        priority.stable_hash(h);
        pick_stats.stable_hash(h);
        offers_due.stable_hash(h);
        offer_open.stable_hash(h);
        spent.stable_hash(h);
        totals.stable_hash(h);
    }
}

impl StableHash for StarState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let StarState { id, at, rarity } = self;
        id.stable_hash(h);
        at.stable_hash(h);
        rarity.stable_hash(h);
    }
}

impl StableHash for Due {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Due { kind, rarity } = self;
        kind.stable_hash(h);
        rarity.stable_hash(h);
    }
}

impl StableHash for OfferState {
    fn stable_hash(&self, h: &mut StateHasher) {
        let OfferState {
            kind,
            rarity,
            candidates,
        } = self;
        kind.stable_hash(h);
        rarity.stable_hash(h);
        candidates.stable_hash(h);
    }
}

impl StableHash for StarField {
    fn stable_hash(&self, h: &mut StateHasher) {
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
        misses.stable_hash(h);
        on_map.stable_hash(h);
        due.stable_hash(h);
        offer.stable_hash(h);
        rerolls_left.stable_hash(h);
        banishes_left.stable_hash(h);
        extra_candidates.stable_hash(h);
        rarity.stable_hash(h);
        rune_pool.stable_hash(h);
        pool.stable_hash(h);
        taken.stable_hash(h);
        banished.stable_hash(h);
        set_counts.stable_hash(h);
    }
}

impl StableHash for HealPickup {
    fn stable_hash(&self, h: &mut StateHasher) {
        let HealPickup { id, at } = self;
        id.stable_hash(h);
        at.stable_hash(h);
    }
}

impl StableHash for ExpertiseOffer {
    fn stable_hash(&self, h: &mut StateHasher) {
        let ExpertiseOffer {
            level,
            kind,
            candidates,
        } = self;
        level.stable_hash(h);
        kind.stable_hash(h);
        candidates.stable_hash(h);
    }
}

impl StableHash for ExpertiseGauge {
    fn stable_hash(&self, h: &mut StateHasher) {
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
        level.stable_hash(h);
        points.stable_hash(h);
        next.stable_hash(h);
        offered.stable_hash(h);
        offer.stable_hash(h);
        taken.stable_hash(h);
        range_pct.stable_hash(h);
        style.stable_hash(h);
        path.stable_hash(h);
    }
}

impl StableHash for Echo {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Echo {
            source,
            target,
            damage,
            damage_type,
        } = self;
        source.stable_hash(h);
        target.stable_hash(h);
        damage.stable_hash(h);
        damage_type.stable_hash(h);
    }
}

impl StableHash for Horde {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Horde {
            // not hashed: the mode's boss times, content fixed for the run.
            boss_ticks: _,
            // not hashed: the mode's no-call times, content fixed for the run.
            no_call_ticks: _,
            // not hashed: the mode's mid-boss hit points, content fixed for the run.
            mid_boss_hp_pct: _,
            spawned,
            used,
            held,
        } = self;
        spawned.stable_hash(h);
        used.stable_hash(h);
        held.stable_hash(h);
    }
}

impl StableHash for State {
    fn stable_hash(&self, h: &mut StateHasher) {
        let State {
            // not hashed: buffers reused from tick to tick, never state.
            scratch: _,
            // not hashed: the run's kinds, content fixed at its start.
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
            // not hashed: derived from `runs` and the run's attacks, which no hash reads.
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
            // not hashed: content, covered by the content hash.
            match_ups: _,
            // not hashed: content, covered by the content hash.
            runes: _,
            // not hashed: content, covered by the content hash.
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
        tick.stable_hash(h);
        rng.stable_hash(h);
        outcome.stable_hash(h);
        player.stable_hash(h);
        boss.stable_hash(h);
        companions.stable_hash(h);
        enemies.stable_hash(h);
        tasks.stable_hash(h);
        runs.stable_hash(h);
        ranged_cooling.stable_hash(h);
        orbs.stable_hash(h);
        zones.stable_hash(h);
        spawned.stable_hash(h);
        next_id.stable_hash(h);
        next_orb_group.stable_hash(h);
        progress.stable_hash(h);
        stars.stable_hash(h);
        heal_pickups.stable_hash(h);
        expertise.stable_hash(h);
        max_attack_speed_pct.stable_hash(h);
        enemy_attack_damage.stable_hash(h);
        max_rune_haste_pct.stable_hash(h);
        horde.stable_hash(h);
        stardust.stable_hash(h);
        regen.stable_hash(h);
        hp_pct.stable_hash(h);
        damage_pct.stable_hash(h);
        crit_seed.stable_hash(h);
        echoes.stable_hash(h);
    }
}

impl StableHash for Outcome {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Outcome::Running => hash_enum!(h, 0),
            Outcome::Cleared => hash_enum!(h, 1),
            Outcome::Failed => hash_enum!(h, 2),
        }
    }
}

impl StableHash for Rarity {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Rarity::Common => hash_enum!(h, 0),
            Rarity::Uncommon => hash_enum!(h, 1),
            Rarity::Rare => hash_enum!(h, 2),
            Rarity::Epic => hash_enum!(h, 3),
            Rarity::Legendary => hash_enum!(h, 4),
            Rarity::Supernova => hash_enum!(h, 5),
        }
    }
}

impl StableHash for OfferKind {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            OfferKind::Star => hash_enum!(h, 0),
            OfferKind::Boss => hash_enum!(h, 1),
        }
    }
}

impl StableHash for ExpertiseOfferKind {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            ExpertiseOfferKind::Common => hash_enum!(h, 0),
            ExpertiseOfferKind::Style => hash_enum!(h, 1),
            ExpertiseOfferKind::Upgrade => hash_enum!(h, 2),
            ExpertiseOfferKind::Capstone => hash_enum!(h, 3),
        }
    }
}

impl StableHash for Rune {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            Rune::Explosion => hash_enum!(h, 0),
            Rune::Thunder => hash_enum!(h, 1),
            Rune::Boomerang => hash_enum!(h, 2),
            Rune::HomingMissiles => hash_enum!(h, 3),
            Rune::Bleed => hash_enum!(h, 4),
            Rune::Poison => hash_enum!(h, 5),
            Rune::Burn => hash_enum!(h, 6),
        }
    }
}

impl StableHash for RangedPool {
    fn stable_hash(&self, h: &mut StateHasher) {
        match self {
            RangedPool::Zone => hash_enum!(h, 0),
            RangedPool::Line => hash_enum!(h, 1),
        }
    }
}

impl StableHash for Streams {
    fn stable_hash(&self, h: &mut StateHasher) {
        let Streams {
            spawn,
            attack,
            loot,
            offer,
            horde,
        } = self;
        spawn.state().stable_hash(h);
        attack.state().stable_hash(h);
        loot.state().stable_hash(h);
        offer.state().stable_hash(h);
        horde.state().stable_hash(h);
    }
}
