//! The offer views the frozen rules of `sim_version` 1 build and read, copied from the live
//! `snapshot` as of 0.4.0 so that the live ones may go (ADR-REPL-2 points 1 to 3). They never
//! leave the crate: `engine/v1_events.rs` converts them to the live types, field by field, and
//! the live types give the wire form. The hash and codec tags of the two kinds are in
//! `state_hash.rs` and `state_codec.rs`, unchanged.

use crate::id::ContentId;
use crate::rewards::Rarity;
use std::sync::Arc;

/// One option of an open offer; `candidate` is a place in its candidates, from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferOption {
    /// Takes the candidate's reward and closes the offer.
    Take { candidate: u8 },
    /// Draws that candidate again, spending a reroll; the offer stays open.
    Reroll { candidate: u8 },
    /// Closes the offer, taking nothing.
    Skip,
    /// Takes the candidate's reward out of the run's pool for good, spending a banish, and
    /// draws its place again; the offer stays open, or closes when nothing is left to offer.
    Banish { candidate: u8 },
}

/// An offer from a falling star (ADR-REPL-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferView {
    pub kind: OfferKind,
    /// The star's rarity, or for a boss offer the highest of the player's tier. A candidate is
    /// of a lower one when the pool has too few left of it.
    pub rarity: Rarity,
    pub candidates: Vec<CandidateView>,
    /// What `Engine::choose` takes, by index: the candidates, then a reroll of each while
    /// rerolls are left and another reward could come, then the skip (not in a boss offer),
    /// then a banish of each while banishes are left.
    pub options: Vec<OfferOption>,
    /// Rerolls and banishes left in the run.
    pub rerolls_left: u32,
    pub banishes_left: u32,
}

/// A reward an offer holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateView {
    /// Its ID in the run's rewards file.
    pub reward: ContentId,
    pub rarity: Rarity,
    /// A random rune: which rune it gives is drawn only when it is taken, so the
    /// frontend shows it face down.
    pub hidden: bool,
    /// A named star: the frontend shows `star` as its name.
    pub named: bool,
    /// The named star's display name; with `named` only.
    pub star: Option<Arc<str>>,
    /// The set a named star belongs to and how far it is; absent for a reward in no set.
    pub constellation: Option<ConstellationProgress>,
}

/// How far a set of named stars is (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstellationView {
    /// Its ID in the run's constellations file.
    pub id: ContentId,
    /// Its name.
    pub name: String,
    /// Members held, carried stars included.
    pub held: u8,
    /// The tier reached, counted from 1; 0 before the first.
    pub tier: u8,
    /// Members the next tier needs; the whole set once the top tier is reached.
    pub needed: u8,
}

/// A candidate that is a member of a set: which one, what is held now and what the next tier
/// needs (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstellationProgress {
    pub set: ContentId,
    /// Members held now, before this candidate is taken.
    pub held: u8,
    /// Members the next tier needs; the whole set once the top tier is reached.
    pub needed: u8,
}

/// What opened an offer (ADR-REPL-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    /// A falling star the main character took; it can be skipped.
    Star,
    /// A boss kill: at the highest rarity of the player's tier, with no skip.
    Boss,
}

/// What an expertise level offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpertiseOfferKind {
    /// Options drawn from the weapon's common pool, with a reroll of each.
    Common,
    /// The weapon's three styles, at the level table's style level: fixed, no reroll.
    Style,
    /// The fixed upgrades of the chosen style, at the table's upgrade levels: no reroll.
    Upgrade,
    /// The fixed capstones of the chosen style, at the table's capstone level: no reroll.
    Capstone,
}
