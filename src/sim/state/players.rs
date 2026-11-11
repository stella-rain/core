//! The party: careers and the player slots (ADR-SIM-7).

use crate::fixed::Fx;
use crate::id::EntityId;
use crate::rng::{SplitMix64, TAG_OFFER, TAG_OFFER_1, TAG_OFFER_2, derive};
use crate::sim::equipment::Equipment;

use super::{ExpertiseGauge, Offers, PlayerState, Progress};

/// The ticks a player stays downed before becoming out for the run (ADR-SIM-7).
pub(crate) const OUT_TIMER_TICKS: u32 = 1200;
/// The ticks a teammate in range must hold interact to revive (ADR-SIM-7).
pub(crate) const REVIVE_HOLD_TICKS: u32 = 180;
/// Hit points restored on revive, in percent of max HP (ADR-SIM-7).
pub(crate) const REVIVE_HP_PCT: u32 = 30;
/// The interaction radius within which a teammate can revive (ADR-SIM-7).
pub(crate) const REVIVE_RANGE: Fx = Fx(12_288);

pub(crate) use crate::id::{MAX_PLAYERS, PlayerIdx};

/// What one player carries through the run besides the player's body: the experience and levels
/// (`None` in a stage without levels), the offer generator and the star offers (`None` in a stage
/// without stars), the expertise gauge, the Stardust earned and the regeneration. One per slot in
/// `Players` (ADR-SIM-7, ADR-SIM-2).
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Career {
    pub progress: Option<Progress>,
    /// The star, expertise and rune offers' generator: `derive(seed, tag)` with the slot's tag
    /// (`TAG_OFFER` for slot 0).
    pub offer: SplitMix64,
    pub offers: Option<Offers>,
    /// The expertise gauge; `None` in a stage whose enemies and boss give no expertise points.
    pub expertise: Option<ExpertiseGauge>,
    /// The Stardust the run has earned, times 10: `Some` in a stage with a Stardust
    /// factor.
    pub stardust: Option<u32>,
    /// Hit points the main character regenerates each second (a perk): set at the start
    /// and raised by rewards.
    pub regen: u32,
    /// The star equipment: its slots are empty in a stage run alone (ADR-SIM-3 point 2).
    pub equipment: Equipment,
}

impl Clone for Career {
    fn clone(&self) -> Career {
        let Career {
            progress,
            offer,
            offers,
            expertise,
            stardust,
            regen,
            equipment,
        } = self;
        Career {
            progress: progress.clone(),
            offer: offer.clone(),
            offers: offers.clone(),
            expertise: expertise.clone(),
            stardust: *stardust,
            regen: *regen,
            equipment: equipment.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Career) {
        let Career {
            progress,
            offer,
            offers,
            expertise,
            stardust,
            regen,
            equipment,
        } = self;
        let Career {
            progress: s_progress,
            offer: s_offer,
            offers: s_offers,
            expertise: s_expertise,
            stardust: s_stardust,
            regen: s_regen,
            equipment: s_equipment,
        } = source;
        progress.clone_from(s_progress);
        offer.clone_from(s_offer);
        offers.clone_from(s_offers);
        expertise.clone_from(s_expertise);
        *stardust = *s_stardust;
        *regen = *s_regen;
        equipment.clone_from(s_equipment);
    }
}

/// The offer generator's tag of slot `me`.
fn offer_tag(me: PlayerIdx) -> u64 {
    match me.get() {
        0 => TAG_OFFER,
        1 => TAG_OFFER_1,
        2 => TAG_OFFER_2,
        _ => unreachable!("{MAX_PLAYERS} slots"),
    }
}

impl Career {
    /// The career of slot `me` at the start of a run seeded `seed`.
    pub(crate) fn new(
        seed: u64,
        me: PlayerIdx,
        progress: Option<Progress>,
        offers: Option<Offers>,
        expertise: Option<ExpertiseGauge>,
        stardust: Option<u32>,
        equipment: Equipment,
    ) -> Career {
        Career {
            progress,
            offer: derive(seed, offer_tag(me)),
            offers,
            expertise,
            stardust,
            regen: 0,
            equipment,
        }
    }

    /// The career of slot `me`, a member who starts or joins, from slot 0's start career `c0`: its
    /// progress, offers, expertise, Stardust and regeneration as they were at the start (the perk's
    /// and a regen Lumine star's), the equipment of `for_new_slot`, its own offer stream.
    pub(crate) fn member(seed: u64, me: PlayerIdx, c0: &Career) -> Career {
        Career {
            regen: c0.regen,
            ..Career::new(
                seed,
                me,
                c0.progress.clone(),
                c0.offers.clone(),
                c0.expertise.clone(),
                c0.stardust,
                c0.equipment.for_new_slot(),
            )
        }
    }
}

/// An absent slot's placeholder: nothing reads or hashes it.
impl Default for Career {
    fn default() -> Career {
        Career {
            progress: None,
            offer: SplitMix64::new(0),
            offers: None,
            expertise: None,
            stardust: None,
            regen: 0,
            equipment: Equipment::default(),
        }
    }
}

/// The players of a run: `1..=MAX_PLAYERS` of them, in the slots a presence mask names. All
/// places are held, the absent ones as copies of the first that nothing reads, so that an index
/// is a plain offset and a steady tick allocates nothing. Slot 0 is always present. The entity
/// numbers 0 to 2 are the slots whether or not one is present (ADR-SIM-7).
///
/// Each slot also holds its `Career` (the level progress, the offer generator, the star offers,
/// the expertise gauge, the Stardust and the regeneration). The state hash and the savepoint write
/// the shared fields, then one presence byte (bit `i` is slot `i`) and, for each present slot in
/// ascending order, its player and its career (ADR-SIM-3, ADR-SAVE-4); an absent slot writes
/// nothing, and a hole in the mask hashes and saves. A party builder supplies a career for every
/// present slot.
#[derive(Debug)]
pub(crate) struct Players {
    items: [PlayerState; MAX_PLAYERS as usize],
    /// The career of each slot; an absent slot holds `Career::default()`, which nothing reads.
    careers: [Career; MAX_PLAYERS as usize],
    /// Bit `i` is set when slot `i` is in the party.
    present: u8,
    /// The generation of each slot's current or last occupant: `leave` adds one, so the next
    /// occupant's entity number matches nothing the leaver owned (ADR-SIM-7). Hashed and saved
    /// for every slot, present or not.
    generations: [u8; MAX_PLAYERS as usize],
}

impl Clone for Players {
    fn clone(&self) -> Players {
        let Players {
            items,
            careers,
            present,
            generations,
        } = self;
        Players {
            items: items.clone(),
            careers: careers.clone(),
            present: *present,
            generations: *generations,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Players) {
        let Players {
            items,
            careers,
            present,
            generations,
        } = self;
        let Players {
            items: s_items,
            careers: s_careers,
            present: s_present,
            generations: s_generations,
        } = source;
        items.clone_from(s_items);
        careers.clone_from(s_careers);
        *present = *s_present;
        *generations = *s_generations;
    }
}

#[cfg(test)]
impl PartialEq for Players {
    fn eq(&self, other: &Players) -> bool {
        self.present == other.present
            && self.generations == other.generations
            && self.iter().eq(other.iter())
            && self.indices().all(|i| self.career(i) == other.career(i))
    }
}

impl Players {
    /// The party of `n` slots (1 to `MAX_PLAYERS`) that grows from `old`'s first player: every
    /// slot a copy of that player, slot 0 keeping its career and each other slot a career of its
    /// own (its offer stream by `offer_tag`) from slot 0's progress, offers, expertise and
    /// Stardust. Slots 1 and 2 take the run's loadout until the per-player setup.
    pub(crate) fn party(n: u8, old: &Players, crit_seed: u64) -> Players {
        assert!((1..=MAX_PLAYERS).contains(&n), "1 to {MAX_PLAYERS} players");
        let first = old.items[0].clone();
        let c0 = old.careers[0].clone();
        let mut out = Players {
            items: [first.clone(), first.clone(), first],
            careers: Default::default(),
            present: (1u8 << n) - 1,
            generations: [0; MAX_PLAYERS as usize],
        };
        out.careers[0] = c0.clone();
        for i in 1..n {
            let me = PlayerIdx::new(i).expect("a slot");
            out.careers[me.get()] = Career::member(crit_seed, me, &c0);
        }
        out
    }

    /// The mask of a party of one.
    const SOLO_MASK: u8 = 0b001;

    /// The party of one that holds slot 0's player and career as they are now: the joiner
    /// template an engine keeps from its start.
    pub(crate) fn template(&self) -> Players {
        Players::solo(self.items[0].clone(), self.careers[0].clone())
    }

    /// Slot `me` joins: a player built as `party` builds slots 1 and 2 from `template`'s first
    /// slot (the run's starting player at level 1, its own offer stream by `offer_tag`), placed
    /// where slot 0 is now and facing as it faces. `me` is a free slot other than 0.
    pub(crate) fn join(&mut self, me: PlayerIdx, template: &Players, crit_seed: u64) {
        assert!(me != PlayerIdx::SOLO && !self.is_present(me), "a free slot");
        let c0 = &template.careers[0];
        let mut player = template.items[0].clone();
        player.at = self.items[0].at;
        player.facing = self.items[0].facing;
        self.items[me.get()] = player;
        self.careers[me.get()] = Career::member(crit_seed, me, c0);
        self.present |= 1 << me.get();
    }

    /// Slot `me` leaves: its bit is cleared and its place is filled as `from_parts` fills an
    /// absent one, which nothing reads or hashes. Slot 0 never leaves.
    pub(crate) fn leave(&mut self, me: PlayerIdx) {
        assert!(
            me != PlayerIdx::SOLO && self.is_present(me),
            "a present slot"
        );
        self.items[me.get()] = self.items[0].clone();
        self.careers[me.get()] = Career::default();
        self.generations[me.get()] = self.generations[me.get()]
            .checked_add(1)
            .expect("a slot below generation 255");
        self.present &= !(1 << me.get());
    }

    pub(crate) fn solo(p: PlayerState, career: Career) -> Players {
        Players {
            items: [p.clone(), p.clone(), p],
            careers: [career, Career::default(), Career::default()],
            present: Self::SOLO_MASK,
            generations: [0; MAX_PLAYERS as usize],
        }
    }

    /// The party `mask` from the players and careers of its present slots, in slot order (the
    /// savepoint's decode). An absent slot is filled as `solo` fills it. `mask` has slot 0 and
    /// `items` and `careers` have one entry per set bit.
    pub(crate) fn from_parts(
        mask: u8,
        generations: [u8; MAX_PLAYERS as usize],
        items: Vec<PlayerState>,
        careers: Vec<Career>,
    ) -> Players {
        assert!(mask & Self::SOLO_MASK != 0 && mask < 1 << MAX_PLAYERS);
        assert!(items.len() == mask.count_ones() as usize && items.len() == careers.len());
        let first = items[0].clone();
        let (mut items, mut careers) = (items.into_iter(), careers.into_iter());
        let mut out = Players {
            items: [first.clone(), first.clone(), first],
            careers: Default::default(),
            present: mask,
            generations,
        };
        for slot in 0..MAX_PLAYERS as usize {
            if mask >> slot & 1 == 1 {
                out.items[slot] = items.next().expect("one player per set bit");
                out.careers[slot] = careers.next().expect("one career per set bit");
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn of(ps: Vec<PlayerState>) -> Option<Players> {
        if ps.is_empty() || ps.len() > usize::from(MAX_PLAYERS) {
            return None;
        }
        let present = (1u8 << ps.len()) - 1;
        let first = ps[0].clone();
        let mut it = ps.into_iter();
        Some(Players {
            items: std::array::from_fn(|_| it.next().unwrap_or_else(|| first.clone())),
            careers: Default::default(),
            present,
            generations: [0; MAX_PLAYERS as usize],
        })
    }

    /// A party of the slots in `mask`, each a copy of `p` (tests).
    #[cfg(test)]
    pub(crate) fn with_mask(mask: u8, p: PlayerState) -> Players {
        assert!(mask & Self::SOLO_MASK != 0, "slot 0 is always present");
        assert!(mask < 1 << MAX_PLAYERS, "a mask of {MAX_PLAYERS} slots");
        Players {
            items: [p.clone(), p.clone(), p],
            careers: Default::default(),
            present: mask,
            generations: [0; MAX_PLAYERS as usize],
        }
    }

    /// This party with slot 0's career taken from `old`: a test that swaps the players of a live
    /// state keeps its progress, offer generator and offers.
    #[cfg(test)]
    pub(crate) fn keeping_career_of(mut self, old: &Players) -> Players {
        self.careers[0] = old.career(PlayerIdx::SOLO).clone();
        // The offers go with the star field in every present slot (the hash and the savepoint
        // check it): the others start with slot 0's offers, and nothing else of its career.
        for me in self.indices().skip(1) {
            self.careers[me.get()].offers = self.careers[0].offers.clone();
        }
        self
    }

    /// The slots in the party, bit `i` for slot `i`.
    #[inline]
    pub(crate) fn mask(&self) -> u8 {
        self.present
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.present.count_ones() as usize
    }

    #[inline]
    pub(crate) fn is_present(&self, i: PlayerIdx) -> bool {
        self.present >> i.get() & 1 == 1
    }

    /// The players of the party, in slot order.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PlayerState> + '_ {
        self.items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.present >> i & 1 == 1)
            .map(|(_, p)| p)
    }

    /// The `PlayerIdx` of each player of the party, in slot order. It holds a copy of the mask,
    /// not the party, so a loop over it may borrow the state mutably.
    pub(crate) fn indices(&self) -> impl Iterator<Item = PlayerIdx> + use<> {
        // Walks the set bits lowest first: slot order, no scan of the absent slots. `present`
        // never holds a bit at or past `MAX_PLAYERS` (the savepoint refuses such a mask).
        let mut rest = self.present;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// The `PlayerIdx` of each party player after slot 0, in slot order: nothing in a party of one.
    /// Slot 0 is always in the party (the savepoint refuses a mask without it), so
    /// `once(SOLO).chain(after_first())` is `indices()` without a test of slot 0.
    #[inline]
    pub(crate) fn after_first(&self) -> impl Iterator<Item = PlayerIdx> + use<> {
        let mut rest = self.present & !1;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// The entity number of slot `me` (ADR-SIM-7): the number its shots, hits and dots carry.
    #[inline]
    pub(crate) fn id(&self, me: PlayerIdx) -> EntityId {
        me.id_at(self.generations[me.get()])
    }

    /// The generation of slot `me`'s current or last occupant.
    #[inline]
    pub(crate) fn generation(&self, me: PlayerIdx) -> u8 {
        self.generations[me.get()]
    }

    /// Sets a slot's generation (tests).
    #[cfg(test)]
    pub(crate) fn set_generation_for_test(&mut self, me: PlayerIdx, generation: u8) {
        self.generations[me.get()] = generation;
    }

    /// The slots that are present and up (alive, not out), bit `i` for slot `i` (ADR-SIM-7).
    #[inline]
    pub(crate) fn up_mask(&self) -> u8 {
        let mut rest = self.present;
        let mut up = 0;
        while rest != 0 {
            let i = rest.trailing_zeros() as usize;
            rest &= rest - 1;
            up |= u8::from(self.items[i].is_up()) << i;
        }
        up
    }

    /// Only slot 0 is present: nearly every run, and the path every per-player step keeps cheap.
    #[inline]
    pub(crate) fn is_solo(&self) -> bool {
        self.present == Self::SOLO_MASK
    }

    /// The slots the world's targeting and bounds read: the up players, or every present slot
    /// when none is up (the tick a party wipes; the run fails at that tick's outcome).
    #[inline]
    pub(crate) fn targets(&self) -> u8 {
        // A party of one, nearly every run: slot 0 is the target up or down.
        if self.present == Self::SOLO_MASK {
            return Self::SOLO_MASK;
        }
        match self.up_mask() {
            0 => self.present,
            up => up,
        }
    }

    /// The slots set in `mask` (a mask of this type's slots), lowest first.
    #[inline]
    pub(crate) fn slots(mask: u8) -> impl Iterator<Item = PlayerIdx> + use<> {
        let mut rest = mask;
        std::iter::from_fn(move || {
            if rest == 0 {
                return None;
            }
            let i = u8::try_from(rest.trailing_zeros()).ok();
            rest &= rest - 1;
            i.and_then(PlayerIdx::new)
        })
    }

    /// Whether any player in the party is alive and not out (ADR-SIM-7).
    #[inline]
    pub(crate) fn any_up(&self) -> bool {
        self.up_count() > 0
    }

    /// How many players in the party are alive and not out (ADR-SIM-7).
    #[inline]
    pub(crate) fn up_count(&self) -> usize {
        self.iter().filter(|p| p.is_up()).count()
    }

    /// The generation of every slot, in slot order (the hash and the savepoint write them).
    #[inline]
    pub(crate) fn generations(&self) -> [u8; MAX_PLAYERS as usize] {
        self.generations
    }

    /// The present slot whose entity number is `id` (ADR-SIM-7): the owner of a shot, a hit or a
    /// dot a player made. `None` for a companion, an enemy or a slot not in the party.
    #[inline]
    pub(crate) fn of_id(&self, id: EntityId) -> Option<PlayerIdx> {
        let (me, generation) = crate::id::player_of(id)?;
        (self.is_present(me) && self.generations[me.get()] == generation).then_some(me)
    }

    /// Players 1 and up that are present, in slot order.
    #[cfg(test)]
    pub(crate) fn others(&self) -> impl Iterator<Item = &PlayerState> + '_ {
        self.iter().skip(1)
    }

    /// The career of player `me`.
    #[inline]
    pub(crate) fn career(&self, me: PlayerIdx) -> &Career {
        debug_assert!(self.is_present(me));
        &self.careers[me.get()]
    }

    #[inline]
    pub(crate) fn career_mut(&mut self, me: PlayerIdx) -> &mut Career {
        debug_assert!(self.is_present(me));
        &mut self.careers[me.get()]
    }

    /// Player `me` and the career of `me` together: a split borrow of the two lists.
    #[inline]
    pub(crate) fn both_mut(&mut self, me: PlayerIdx) -> (&mut PlayerState, &mut Career) {
        debug_assert!(self.is_present(me));
        (&mut self.items[me.get()], &mut self.careers[me.get()])
    }

    /// The careers of players 1 and up that are present, in slot order, like `others`.
    #[cfg(test)]
    pub(crate) fn other_careers(&self) -> impl Iterator<Item = &Career> + '_ {
        self.careers
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, _)| self.present >> i & 1 == 1)
            .map(|(_, c)| c)
    }

    /// All three careers, absent slots included (tests).
    #[cfg(test)]
    pub(crate) fn careers_mut_for_test(&mut self) -> &mut [Career; MAX_PLAYERS as usize] {
        &mut self.careers
    }
}

impl std::ops::Index<PlayerIdx> for Players {
    type Output = PlayerState;
    #[inline]
    fn index(&self, i: PlayerIdx) -> &PlayerState {
        debug_assert!(self.is_present(i));
        &self.items[i.get()]
    }
}

impl std::ops::IndexMut<PlayerIdx> for Players {
    #[inline]
    fn index_mut(&mut self, i: PlayerIdx) -> &mut PlayerState {
        debug_assert!(self.is_present(i));
        &mut self.items[i.get()]
    }
}
