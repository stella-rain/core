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
/// level. A kill by the player raises both (`slots::credit_kills`); a replace by a star sharing no
/// option halves the level and resets the kills, a same-option switch keeps both (`slots::apply`).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct EquipSlot {
    pub kind: SlotKind,
    pub held: Option<Held>,
    pub level: u8,
    pub expertise: u32,
}

/// Every per-player equipment field.
#[derive(Debug, Default)]
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

impl Clone for Equipment {
    fn clone(&self) -> Equipment {
        let Equipment {
            inventory,
            slots,
            lumine,
            starlight,
            tree_points,
        } = self;
        Equipment {
            inventory: inventory.clone(),
            slots: slots.clone(),
            lumine: lumine.clone(),
            starlight: *starlight,
            tree_points: *tree_points,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Equipment) {
        let Equipment {
            inventory,
            slots,
            lumine,
            starlight,
            tree_points,
        } = self;
        let Equipment {
            inventory: s_inventory,
            slots: s_slots,
            lumine: s_lumine,
            starlight: s_starlight,
            tree_points: s_tree_points,
        } = source;
        inventory.clone_from(s_inventory);
        slots.clone_from(s_slots);
        lumine.clone_from(s_lumine);
        *starlight = *s_starlight;
        *tree_points = *s_tree_points;
    }
}

impl Equipment {
    /// What a member who joins takes from slot 0's equipment: the slots and the Lumine slot as
    /// they are, an empty inventory, no Starlight and no tree points (ADR-SAVE-7 point 2).
    pub(crate) fn for_new_slot(&self) -> Equipment {
        Equipment {
            inventory: Vec::new(),
            slots: self.slots.clone(),
            lumine: self.lumine.clone(),
            starlight: 0,
            tree_points: 0,
        }
    }
}
