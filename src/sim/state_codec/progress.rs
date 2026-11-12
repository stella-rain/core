//! The savepoint encoding of the run's progress, stars, offers, healing, expertise and horde.

use super::shared::{MAX_CONTENT_LEN, cap_u32};
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec};
use crate::sim::state::*;

/// The most places a list of `u8` places into a content file holds.
const MAX_PLACES: u32 = 256;

impl Encode for Progress {
    fn encode(&self, w: &mut Writer) {
        let Progress {
            level,
            xp,
            // not saved: the run stats' rules are content: `decode_state` copies them from the fresh engine
            stats: _,
            priority,
            spent,
            totals,
            set_tenths,
        } = self;
        level.encode(w);
        xp.encode(w);
        priority.encode(w);
        spent.encode(w);
        totals.encode(w);
        set_tenths.encode(w);
    }
}

impl Decode for Progress {
    /// `stats` is left empty: `decode_state` copies it from the fresh engine.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let progress = Progress {
            level: Decode::decode(r)?,
            xp: Decode::decode(r)?,
            stats: Vec::new(),
            priority: decode_vec(r, MAX_PLACES)?,
            spent: decode_vec(r, MAX_PLACES)?,
            totals: Decode::decode(r)?,
            set_tenths: Decode::decode(r)?,
        };
        // A weapon's stat set is at most `MAX_STEP` percent of an effect, in tenths.
        let most = crate::run_stats::MAX_STEP.saturating_mul(10);
        // Hit points of a set are whole and live in `totals`, never in the tenths.
        if progress.set_tenths.iter().any(|t| *t > most)
            || progress.set_tenths[crate::run_stats::StatEffect::MaxHp as usize] != 0
        {
            return Err(DecodeError::Invalid("a stat set value past its bound"));
        }
        Ok(progress)
    }
}

/// The star field's shared part: the `Option` tag, then the misses, the stars on the map, the
/// rarity, the rune pool and the reward pool. The offers are each player's, in the blocks.
pub(super) fn encode_star_field(stars: &Option<StarField>, w: &mut Writer) {
    let Some(f) = stars else {
        w.u8(0);
        return;
    };
    w.u8(1);
    let StarField {
        misses,
        on_map,
        rarity,
        rune_pool,
        pool,
    } = f;
    misses.encode(w);
    on_map.encode(w);
    rarity.encode(w);
    rune_pool.encode(w);
    pool.encode(w);
}

/// The star field in the order of `encode_star_field`.
pub(super) fn decode_star_field(r: &mut Reader<'_>) -> Result<Option<StarField>, DecodeError> {
    match r.u8()? {
        0 => return Ok(None),
        1 => {}
        _ => return Err(DecodeError::Invalid("option tag")),
    }
    Ok(Some(StarField {
        misses: Decode::decode(r)?,
        on_map: decode_vec(r, cap_u32(usize::from(crate::limits::MAX_STARS_ON_MAP)))?,
        rarity: Decode::decode(r)?,
        rune_pool: Decode::decode(r)?,
        pool: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
    }))
}

impl Encode for Offers {
    fn encode(&self, w: &mut Writer) {
        let Offers { set_counts } = self;
        set_counts.encode(w);
    }
}

impl Decode for Offers {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Offers {
            set_counts: decode_vec(r, cap_u32(crate::limits::MAX_REWARDS))?,
        })
    }
}

impl Encode for StarState {
    fn encode(&self, w: &mut Writer) {
        let StarState { id, at, rarity } = self;
        id.encode(w);
        at.encode(w);
        rarity.encode(w);
    }
}

impl Decode for StarState {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(StarState {
            id: Decode::decode(r)?,
            at: Decode::decode(r)?,
            rarity: Decode::decode(r)?,
        })
    }
}

impl Encode for HealPickup {
    fn encode(&self, w: &mut Writer) {
        let HealPickup { id, at } = self;
        id.encode(w);
        at.encode(w);
    }
}

impl Decode for HealPickup {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(HealPickup {
            id: Decode::decode(r)?,
            at: Decode::decode(r)?,
        })
    }
}

impl Encode for ExpertiseGauge {
    fn encode(&self, w: &mut Writer) {
        let ExpertiseGauge {
            level,
            points,
            next,
            range_pct,
            style,
            path,
        } = self;
        level.encode(w);
        points.encode(w);
        next.encode(w);
        range_pct.encode(w);
        style.encode(w);
        path.encode(w);
    }
}

impl Decode for ExpertiseGauge {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ExpertiseGauge {
            level: Decode::decode(r)?,
            points: Decode::decode(r)?,
            next: Decode::decode(r)?,
            range_pct: Decode::decode(r)?,
            style: Decode::decode(r)?,
            path: decode_vec(r, MAX_PLACES)?,
        })
    }
}

impl Encode for Horde {
    fn encode(&self, w: &mut Writer) {
        let Horde {
            // not saved: the mode's boss times are content: `decode_state` copies them from the fresh engine
            boss_ticks: _,
            // not saved: the mode's no-call times are content: `decode_state` copies them from the fresh engine
            no_call_ticks: _,
            // not saved: the mid-bosses' hit points are content: `decode_state` copies them from the fresh engine
            mid_boss_hp_pct: _,
            spawned,
            used,
            held,
        } = self;
        spawned.encode(w);
        used.encode(w);
        held.encode(w);
    }
}

impl Decode for Horde {
    /// The times are left empty: `decode_state` copies them from the fresh engine.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Horde {
            boss_ticks: Vec::new(),
            no_call_ticks: Vec::new(),
            mid_boss_hp_pct: Vec::new(),
            spawned: Decode::decode(r)?,
            used: decode_vec(r, MAX_CONTENT_LEN)?,
            held: decode_vec(r, MAX_CONTENT_LEN)?,
        })
    }
}
