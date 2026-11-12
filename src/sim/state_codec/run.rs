//! The savepoint encoding of the run state itself: echoes, generators and `State`.

use super::progress::{decode_star_field, encode_star_field};
use super::shared::{MAX_CONTENT_LEN, cap_u32};
use crate::id::EntityId;
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec};
use crate::sim::state::*;

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

/// Four `u64`, each run generator's state, in the order the hash writes them: `spawn, attack,
/// loot, horde`. Each player's offer generator is in its block.
fn encode_streams(rng: &Streams, w: &mut Writer) {
    let Streams {
        spawn,
        attack,
        loot,
        horde,
    } = rng;
    spawn.encode(w);
    attack.encode(w);
    loot.encode(w);
    horde.encode(w);
}

/// The generators in the order of `encode_streams`.
fn decode_streams(r: &mut Reader<'_>) -> Result<Streams, DecodeError> {
    Ok(Streams {
        spawn: Decode::decode(r)?,
        attack: Decode::decode(r)?,
        loot: Decode::decode(r)?,
        horde: Decode::decode(r)?,
    })
}

/// The runs one attacker can have at once: one for each `u8` slot.
const MAX_RUN_SLOTS: u32 = 256;

impl Encode for State {
    fn encode(&self, w: &mut Writer) {
        self.encode_fields(w);
    }
}

impl State {
    fn encode_fields(&self, w: &mut Writer) {
        let State {
            // not saved: buffers reused from tick to tick, never state.
            scratch: _,
            // not saved: the run's kinds are content; `decode_state` takes them from the fresh engine.
            kinds: _,
            tick,
            rng,
            outcome,
            players,
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
            stars,
            heal_pickups,
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
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
        } = self;
        tick.encode(w);
        encode_streams(rng, w);
        outcome.encode(w);
        boss.encode(w);
        companions.encode(w);
        enemies.encode(w);
        tasks.encode(w);
        runs.encode(w);
        ranged_cooling.encode(w);
        orbs.encode(w);
        zones.encode(w);
        spawned.encode(w);
        EntityId(*next_id).encode(w);
        next_orb_group.encode(w);
        encode_star_field(stars, w);
        heal_pickups.encode(w);
        max_attack_speed_pct.encode(w);
        enemy_attack_damage.encode(w);
        max_rune_haste_pct.encode(w);
        horde.encode(w);
        hp_pct.encode(w);
        damage_pct.encode(w);
        crit_seed.encode(w);
        echoes.encode(w);
        // The presence byte, then the block of each present player in slot order, as the hash.
        w.u8(players.mask());
        for g in players.generations() {
            w.u8(g);
        }
        for me in players.indices() {
            // Fail closed: the star field and each present player's offers go together.
            assert_eq!(
                stars.is_some(),
                players.career(me).offers.is_some(),
                "the star field and a present player's offers go together"
            );
            players[me].encode(w);
            let Career {
                progress,
                offer,
                offers,
                expertise,
                stardust,
                regen,
                equipment,
            } = players.career(me);
            progress.encode(w);
            offer.encode(w);
            offers.encode(w);
            expertise.encode(w);
            stardust.encode(w);
            regen.encode(w);
            equipment.encode(w);
        }
    }
}

impl State {
    /// The state in `r`, with the fields it does not save (the content's tables and buffers)
    /// taken from `fresh`, the state of an engine started from the same setup and content. The
    /// lists are only as long as the format allows: `decode_state` checks them against `fresh`,
    /// and fills the index and the content-sized fields of every present player's `progress` and
    /// of `horde` in. The party is the one `fresh` has: a savepoint of another one is refused.
    pub(crate) fn decode_over(r: &mut Reader<'_>, fresh: &State) -> Result<State, DecodeError> {
        let state = Self::decode_fields(r, fresh)?;
        // The next entity number is past the player slots, whatever the bytes say.
        if state.next_id < u32::from(crate::id::MAX_PLAYERS) {
            return Err(DecodeError::Invalid("next entity id in the player slots"));
        }
        // ... and short of the numbers a rejoined player has (`id::MAX_ENTITY_NUMBER`).
        if state.next_id >= crate::id::MAX_ENTITY_NUMBER {
            return Err(DecodeError::Invalid(
                "next entity id past the entity numbers",
            ));
        }
        Ok(state)
    }

    fn decode_fields(r: &mut Reader<'_>, fresh: &State) -> Result<State, DecodeError> {
        let companions_cap = cap_u32(crate::validate::limits::MAX_COMPANIONS);
        let enemies_cap = u32::from(crate::limits::MAX_ENEMY_CAP);
        let mut state = State {
            scratch: Default::default(),
            kinds: fresh.kinds.clone(),
            tick: Decode::decode(r)?,
            rng: decode_streams(r)?,
            outcome: Decode::decode(r)?,
            // The party comes last in the bytes; `fresh`'s stands in until then.
            players: fresh.players.clone(),
            boss: Decode::decode(r)?,
            companions: decode_vec(r, companions_cap)?,
            enemies: decode_vec(r, enemies_cap)?,
            tasks: decode_vec(r, crate::limits::MAX_ATTACK_TASKS)?,
            runs: decode_vec(r, (enemies_cap + companions_cap + 1) * MAX_RUN_SLOTS)?,
            move_index: Default::default(),
            ranged_cooling: decode_vec(r, crate::limits::MAX_RANGED_ATTACKERS)?,
            orbs: decode_vec(r, crate::limits::MAX_ORBS_ALIVE)?,
            zones: decode_vec(r, crate::limits::MAX_ZONES_ALIVE)?,
            spawned: decode_vec(r, MAX_CONTENT_LEN)?,
            next_id: EntityId::decode(r)?.0,
            next_orb_group: Decode::decode(r)?,
            stars: decode_star_field(r)?,
            heal_pickups: match r.u8()? {
                0 => None,
                1 => Some(decode_vec(
                    r,
                    u32::from(crate::validate::limits::MAX_HEAL_PICKUPS),
                )?),
                _ => return Err(DecodeError::Invalid("option tag")),
            },
            match_ups: fresh.match_ups,
            runes: fresh.runes.clone(),
            stun: fresh.stun,
            max_attack_speed_pct: Decode::decode(r)?,
            enemy_attack_damage: Decode::decode(r)?,
            max_rune_haste_pct: Decode::decode(r)?,
            horde: Decode::decode(r)?,
            hp_pct: Decode::decode(r)?,
            damage_pct: Decode::decode(r)?,
            crit_seed: Decode::decode(r)?,
            echoes: decode_vec(r, cap_u32(crate::limits::MAX_ECHOES_PER_TICK))?,
        };
        // The presence byte: slot 0 always, never a slot past the last, and the party the setup
        // started.
        let mask = r.u8()?;
        if mask & 1 == 0 || mask >> crate::id::MAX_PLAYERS != 0 {
            return Err(DecodeError::Invalid("player mask"));
        }
        if mask != fresh.players.mask() {
            return Err(DecodeError::Invalid("player mask differs from the setup"));
        }
        // A generation is state, not a check: any byte restores.
        let mut generations = [0u8; crate::id::MAX_PLAYERS as usize];
        for g in &mut generations {
            *g = Decode::decode(r)?;
        }
        let mut items = Vec::new();
        let mut careers = Vec::new();
        for _ in 0..mask.count_ones() {
            items.push(Decode::decode(r)?);
            let career = Career {
                progress: Decode::decode(r)?,
                offer: Decode::decode(r)?,
                offers: Decode::decode(r)?,
                expertise: Decode::decode(r)?,
                stardust: Decode::decode(r)?,
                regen: Decode::decode(r)?,
                equipment: Decode::decode(r)?,
            };
            if career.offers.is_some() != state.stars.is_some() {
                return Err(DecodeError::Invalid(
                    "offers presence differs from the star field",
                ));
            }
            careers.push(career);
        }
        state.players = Players::from_parts(mask, generations, items, careers);
        Ok(state)
    }
}
