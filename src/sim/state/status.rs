//! The rule, status and damage-over-time states of an entity (ADR-SIM-3).

use crate::attack::DamageType;
use crate::id::{ContentId, EntityId};
use crate::inline::InlineVec;
use crate::validate::{self};
use std::sync::LazyLock;

/// The rule states of an agent or a boss: at most `MAX_RULES`, which the validator holds.
pub(crate) type Rules = InlineVec<RuleState, { validate::limits::MAX_RULES }>;
/// The statuses on one entity: one of each kind at most (`add_status`, `add_stun`, `add_dot` add
/// a stack to the one there), so no more than there are kinds.
pub(crate) type Statuses = InlineVec<StatusState, { StatusKind::COUNT }>;

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct RuleState {
    pub cooldown_left: u32,
    /// Firings left; `UNLIMITED` if there is no limit.
    pub charges_left: u32,
    pub fired: bool,
}

/// The statuses of this version (ADR-SIM-4): the three a skill can apply, and bleed, poison and
/// burn, which only their runes apply. The IDs in the registry are the names of the
/// first three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// More damage dealt.
    AtkUp,
    /// More damage taken.
    Vulnerable,
    /// Slower movement.
    Slow,
    /// Damage over time, in stacks.
    Bleed,
    /// Damage over time, in stacks, of its own beside bleed's.
    Poison,
    /// Damage over time, in stacks, of its own beside bleed's.
    Burn,
    /// The enemy stops what it was doing for its ticks. Not a skill's status.
    Stun,
}

/// The IDs of `StatusKind::ALL`, in that order, which is the order of the discriminants
/// (`every_status_kind_is_listed`). Constants, never hashed.
static STATUS_IDS: LazyLock<[ContentId; StatusKind::COUNT]> =
    LazyLock::new(|| StatusKind::ALL.map(|k| ContentId::from(k.name())));

impl StatusKind {
    /// The statuses a skill can apply: the damage-over-time ones are not among them.
    pub const FROM_SKILLS: [StatusKind; 3] =
        [StatusKind::AtkUp, StatusKind::Vulnerable, StatusKind::Slow];

    /// The status a skill can apply whose registry ID is `name`.
    pub fn from_name(name: &str) -> Option<StatusKind> {
        StatusKind::FROM_SKILLS
            .into_iter()
            .find(|k| k.name() == name)
    }

    /// Every kind. `COUNT` sizes the lists of statuses; a kind added without a place here fails
    /// the test `every_status_kind_is_listed`.
    pub const ALL: [StatusKind; 7] = [
        StatusKind::AtkUp,
        StatusKind::Vulnerable,
        StatusKind::Slow,
        StatusKind::Bleed,
        StatusKind::Poison,
        StatusKind::Burn,
        StatusKind::Stun,
    ];
    pub const COUNT: usize = StatusKind::ALL.len();

    /// The status as a content ID, a refcount bump of the shared table: an event or a snapshot
    /// that names a status allocates nothing.
    pub fn id(self) -> ContentId {
        STATUS_IDS[self as usize].clone()
    }

    pub fn name(self) -> &'static str {
        match self {
            StatusKind::AtkUp => "atk_up",
            StatusKind::Vulnerable => "vulnerable",
            StatusKind::Slow => "slow",
            StatusKind::Bleed => "bleed",
            StatusKind::Poison => "poison",
            StatusKind::Burn => "burn",
            StatusKind::Stun => "stun",
        }
    }
}

/// One status on one entity. Applying it again refreshes the duration and adds a stack.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StatusState {
    pub kind: StatusKind,
    /// Who applied it last.
    pub source: EntityId,
    pub remaining: u32,
    /// The ticks of the application that set `remaining`, for the snapshot: a
    /// refresh with a longer duration sets it, a shorter one leaves both alone.
    pub total: u32,
    pub stacks: u16,
    /// Percent per stack.
    pub value_pct: u16,
    /// A damage-over-time status's own state; `None` for every other status.
    pub dot: Option<DotState>,
}

/// The filler of the unused places of a list of statuses; never read.
impl Default for StatusState {
    fn default() -> Self {
        StatusState {
            kind: StatusKind::AtkUp,
            source: EntityId(0),
            remaining: 0,
            total: 0,
            stacks: 0,
            value_pct: 0,
            dot: None,
        }
    }
}

/// What a damage-over-time status deals and when: its damage a stack
/// and its type, from the content that applied it; the atk-up and type bonus of its source,
/// fixed when it was last applied, as an attack's are when it starts; its interval,
/// and the ticks to its next tick, at least 1.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct DotState {
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub atk_up_pct: u32,
    pub type_bonus_pct: u32,
    pub interval: u32,
    pub next_in: u32,
}
