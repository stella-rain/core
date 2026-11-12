//! The savepoint encoding of the equipment types.

use super::shared::cap_u32;
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec};
use crate::sim::equipment::{EquipSlot, Equipment, Held, SlotKind};

impl Encode for Held {
    fn encode(&self, w: &mut Writer) {
        match self {
            Held::Star(place) => {
                w.u8(0);
                place.encode(w);
            }
            Held::Rune(rune) => {
                w.u8(1);
                rune.encode(w);
            }
        }
    }
}

impl Decode for Held {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(Held::Star(Decode::decode(r)?)),
            1 => Ok(Held::Rune(Decode::decode(r)?)),
            _ => Err(DecodeError::Invalid("held tag")),
        }
    }
}

impl Encode for SlotKind {
    fn encode(&self, w: &mut Writer) {
        match self {
            SlotKind::Category(index) => {
                w.u8(0);
                index.encode(w);
            }
            SlotKind::Rune => w.u8(1),
            SlotKind::Lumine => w.u8(2),
        }
    }
}

impl Decode for SlotKind {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(SlotKind::Category(Decode::decode(r)?)),
            1 => Ok(SlotKind::Rune),
            2 => Ok(SlotKind::Lumine),
            _ => Err(DecodeError::Invalid("slot kind tag")),
        }
    }
}

impl Encode for EquipSlot {
    fn encode(&self, w: &mut Writer) {
        let EquipSlot {
            kind,
            held,
            level,
            expertise,
        } = self;
        kind.encode(w);
        held.encode(w);
        level.encode(w);
        expertise.encode(w);
    }
}

impl Decode for EquipSlot {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let slot = EquipSlot {
            kind: Decode::decode(r)?,
            held: Decode::decode(r)?,
            level: Decode::decode(r)?,
            expertise: Decode::decode(r)?,
        };
        if slot.level > crate::limits::MAX_SLOT_LEVEL {
            return Err(DecodeError::Invalid("slot level over the cap"));
        }
        Ok(slot)
    }
}

impl Encode for Equipment {
    fn encode(&self, w: &mut Writer) {
        let Equipment {
            inventory,
            slots,
            lumine,
            starlight,
            tree_points,
        } = self;
        inventory.encode(w);
        slots.encode(w);
        lumine.encode(w);
        starlight.encode(w);
        tree_points.encode(w);
    }
}

impl Decode for Equipment {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let equipment = Equipment {
            inventory: decode_vec(r, cap_u32(crate::limits::MAX_INVENTORY))?,
            slots: decode_vec(r, cap_u32(crate::limits::MAX_EQUIP_SLOTS))?,
            lumine: Decode::decode(r)?,
            starlight: Decode::decode(r)?,
            tree_points: Decode::decode(r)?,
        };
        if equipment
            .slots
            .iter()
            .any(|s| matches!(s.kind, SlotKind::Lumine))
        {
            return Err(DecodeError::Invalid("a Lumine kind among the slots"));
        }
        if equipment
            .lumine
            .as_ref()
            .is_some_and(|s| !matches!(s.kind, SlotKind::Lumine))
        {
            return Err(DecodeError::Invalid("the Lumine slot is not a Lumine kind"));
        }
        Ok(equipment)
    }
}
