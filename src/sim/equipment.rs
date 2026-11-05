//! A player's star equipment (ADR-SIM-3 point 2, ADR-SAVE-4 point 7): the inventory, the slots
//! with their level and expertise, the Lumine slot, Starlight and the unspent weapon-tree points.
//! One field of the career carries all of it, so a rule that fills it adds no field and no hash
//! move. An offers run carries it empty. No rule reads or writes it yet.

use crate::rune::Rune;

/// What an inventory place or a slot holds.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum Held {
    /// A star: a place in the run's rewards file, as `OfferState.candidates` is.
    Star(u16),
    Rune(Rune),
}

/// Which slot it is.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum SlotKind {
    /// A category slot: the pack's category index.
    Category(u8),
    Rune,
    /// The Lumine slot, present only when the setup names a Lumine star.
    Lumine,
}

/// One slot: what it holds, its level (0 while it never held one) and the kills toward its next
/// level.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct EquipSlot {
    pub kind: SlotKind,
    pub held: Option<Held>,
    pub level: u8,
    pub expertise: u32,
}

/// Every per-player equipment field.
#[derive(Clone, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Equipment {
    pub inventory: Vec<Held>,
    pub slots: Vec<EquipSlot>,
    /// Present exactly when the run's setup names a Lumine star.
    pub lumine: Option<EquipSlot>,
    /// The in-run currency (rules add with `saturating_add`, ADR-SIM-5 point 8).
    pub starlight: u32,
    /// Unspent weapon-tree points.
    pub tree_points: u32,
}
