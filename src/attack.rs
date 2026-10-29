//! Attacks (ADR-055, amending tier 3 of ADR-036): an attack is an ordered list of phases, each
//! lasting a fixed number of ticks, with its own hit box, base damage and the emitter nodes it
//! spawns. Emitter nodes are trees whose parameters are small fixed-point expressions. Presets
//! are registered content IDs; a stage refers to one with arguments, or holds an inline copy it
//! may edit freely.
//!
//! Angles are in 1/256 degree; speeds and lengths in 1/256 pixel (per tick for speeds; the
//! units of `Fx`). Field, node and operator names are placeholders until the freeze (ADR-035).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fixed::Fx;
use crate::id::ContentId;

/// A preset by ID with its arguments, or an inline definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AttackRef {
    Preset {
        id: ContentId,
        #[serde(default)]
        #[schemars(length(max = crate::validate::limits::MAX_PRESET_ARGS))]
        args: Vec<i32>,
    },
    /// The phases, in order.
    Inline(
        #[schemars(length(min = 1, max = crate::validate::limits::MAX_ATTACK_PHASES))]
        Vec<AttackPhase>,
    ),
}

/// One phase of an attack (ADR-055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttackPhase {
    pub kind: PhaseKind,
    /// How long the phase lasts: a constant, never an expression, so that an attack's length
    /// and its telegraph are known before it runs.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_PHASE_TICKS))]
    pub ticks: u32,
    /// The area the phase covers, attached to the attacker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_box: Option<HitBox>,
    /// Base damage of the phase's hits and of the zones it spawns (ADR-056). Absent, it is
    /// what the attacker's own attack does: the main character's `atk`, a `shot` skill's
    /// `damage`, or 100 (1 at the design values, ADR-103) for an enemy or the boss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_DAMAGE))]
    pub damage: Option<u32>,
    /// The damage type of the phase's hits and of the zones it spawns (ADR-100); absent, they
    /// are neutral and never get a match-up.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<DamageType>,
    /// An active phase only: a target already hit may be hit again after this many ticks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_PHASE_TICKS))]
    pub rehit_ticks: Option<u32>,
    /// Emitter nodes started on the phase's first tick, carried out in order (see `sim`'s
    /// attacks for what each one does).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawn: Vec<Emitter>,
    /// The attacker's movement speed during the phase, in percent; 0 roots it.
    #[serde(default = "full_speed", skip_serializing_if = "is_full_speed")]
    #[schemars(range(min = 0, max = 100))]
    pub move_pct: u8,
    /// A powerful attack: its hits on the main character, by hit box or by the zones the phase
    /// spawns, break a charge in progress (core#83).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub breaks_charge: bool,
    /// An active phase of an enemy's or the boss's attack only: the attacker moves along the
    /// attack's locked facing over the phase, hit box and boss parts with it (DS11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
}

/// A phase that carries its attacker forward: it covers `distance` over the phase's ticks, held
/// to the map. The phase's `move_pct` is 0, so that the walk does not add to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dash {
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_DASH_DISTANCE))]
    pub distance: Fx,
}

fn full_speed() -> u8 {
    100
}

fn is_full_speed(v: &u8) -> bool {
    *v == 100
}

/// The damage types, a closed list (ADR-100): three attack types, which are physical, and the
/// elements. A list for the prototype; changing it is a schema change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Slash,
    Pierce,
    Strike,
    Pyro,
    Hydro,
    Electro,
    Cryo,
    Anemo,
    Geo,
    Dendro,
}

impl DamageType {
    /// Every type, attack types first.
    pub const ALL: [DamageType; 10] = [
        DamageType::Slash,
        DamageType::Pierce,
        DamageType::Strike,
        DamageType::Pyro,
        DamageType::Hydro,
        DamageType::Electro,
        DamageType::Cryo,
        DamageType::Anemo,
        DamageType::Geo,
        DamageType::Dendro,
    ];

    /// The type's place in `ALL`, from 0.
    pub fn index(self) -> usize {
        self as usize
    }

    /// An attack type (Slash, Pierce, Strike) rather than an element.
    pub fn is_physical(self) -> bool {
        matches!(
            self,
            DamageType::Slash | DamageType::Pierce | DamageType::Strike
        )
    }
}

/// A blanket resist: every attack type, or every element (ADR-100).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blanket {
    Physical,
    Elements,
}

impl Blanket {
    /// Whether the blanket covers `ty`.
    pub fn covers(self, ty: DamageType) -> bool {
        match self {
            Blanket::Physical => ty.is_physical(),
            Blanket::Elements => !ty.is_physical(),
        }
    }
}

/// What an enemy, an elite, a boss or a boss part is weak and strong against (ADR-100). The
/// validator holds each type at most once in each list, and never in both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weak: Vec<DamageType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strong: Vec<DamageType>,
    /// At most one blanket resist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resist: Option<Blanket>,
}

impl Profile {
    /// The one match-up a hit of `ty` gets: a listed weakness, else a listed strength, else the
    /// blanket, else none. A neutral hit gets none.
    pub fn match_up(&self, ty: Option<DamageType>) -> MatchUp {
        let Some(ty) = ty else {
            return MatchUp::None;
        };
        if self.weak.contains(&ty) {
            MatchUp::Weak
        } else if self.strong.contains(&ty) {
            MatchUp::Strong
        } else if self.resist.is_some_and(|b| b.covers(ty)) {
            MatchUp::Resisted
        } else {
            MatchUp::None
        }
    }
}

/// The match-up a hit got (ADR-100), which the `Hit` event reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchUp {
    Weak,
    Strong,
    Resisted,
    #[default]
    None,
}

impl MatchUp {
    pub fn is_none(&self) -> bool {
        *self == MatchUp::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    /// The telegraph: its hit box is the warned area, and it deals no damage.
    WindUp,
    /// Its hit box deals damage.
    Active,
    Recovery,
}

/// Where a hit box or a zone is anchored (core Z1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The attacker's own spot: a hit box follows its attacker, a zone stays where the attacker
    /// was when the zone was placed.
    #[default]
    Attacker,
    /// The target's spot: where the target stood when the attack began, so that a target that
    /// leaves during the wind-up leaves the area too.
    Target,
}

impl Anchor {
    pub fn is_attacker(&self) -> bool {
        *self == Anchor::Attacker
    }
}

impl HitBox {
    /// Where the hit box is anchored.
    pub fn anchor(&self) -> Anchor {
        match self {
            HitBox::Circle { anchor, .. }
            | HitBox::Arc { anchor, .. }
            | HitBox::Line { anchor, .. } => *anchor,
        }
    }
}

/// A hit box, in the attacker's own frame: `offset` is forward along the attacker's facing
/// and to its right, from the `anchor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HitBox {
    Circle {
        #[serde(default, skip_serializing_if = "Anchor::is_attacker")]
        anchor: Anchor,
        #[serde(default)]
        offset: Offset,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        radius: Fx,
    },
    /// A circle sector centred on the facing, `half_angle` to each side.
    Arc {
        #[serde(default, skip_serializing_if = "Anchor::is_attacker")]
        anchor: Anchor,
        #[serde(default)]
        offset: Offset,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        radius: Fx,
        half_angle: i32,
    },
    /// A rectangle from its starting point (the anchor and `offset`) along the facing turned by
    /// `angle`: `length` long and `width` wide, half of it to each side.
    Line {
        #[serde(default, skip_serializing_if = "Anchor::is_attacker")]
        anchor: Anchor,
        #[serde(default)]
        offset: Offset,
        /// Turn from the facing, in 1/256 degree; positive turns toward the right.
        #[serde(default)]
        angle: i32,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        length: Fx,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        width: Fx,
    },
}

/// The area of a zone (core Z1), laid out like a hit box but placed by its emitter node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZoneShape {
    Circle {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        radius: Fx,
    },
    /// A circle sector centred on the zone's direction, `half_angle` to each side.
    Arc {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        radius: Fx,
        half_angle: i32,
    },
    /// A rectangle from the zone's centre along its direction.
    Line {
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        length: Fx,
        #[schemars(range(min = 1, max = crate::validate::limits::MAX_RADIUS))]
        width: Fx,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    #[serde(default)]
    pub forward: Fx,
    #[serde(default)]
    pub right: Fx,
}

impl Offset {
    pub fn is_zero(&self) -> bool {
        *self == Offset::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Emitter {
    /// Runs `body` `count` times (at most 100). The next round starts `interval_ticks` ticks
    /// after the body of the last one ends.
    Repeat {
        count: Expr,
        interval_ticks: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times at once, each round turned a full turn / `count` further than
    /// the one before.
    Ring {
        count: Expr,
        body: Vec<Emitter>,
    },
    /// Runs `body` `count` times at once, fanned evenly across `angle` and centred on the
    /// direction of the attack.
    Spread {
        count: Expr,
        angle: Expr,
        body: Vec<Emitter>,
    },
    /// Turns the direction of everything in `body` by `angle`.
    Rotate {
        angle: Expr,
        body: Vec<Emitter>,
    },
    Wait {
        ticks: Expr,
    },
    /// A slow projectile with a required lifetime (core Z2, DS10): it flies for `ticks` ticks
    /// (1 to `limits::MAX_ORB_TICKS`), and then `burst` runs from where it ended, as an attack of
    /// its own. A `breakable` orb is destroyed, with no burst, by the main character's hit boxes.
    /// Enemy attacks only; the burst holds only `zone`, `ring`, `spread`, `repeat`, `rotate` and
    /// `wait` nodes. At most `MAX_ORBS_PER_OWNER` orbs per attacker and `MAX_ORBS_ALIVE` in all;
    /// one over a cap is dropped.
    Orb {
        speed: Expr,
        direction: Direction,
        /// Where the orbs spawn from: the attacker, or the target as it is when the node fires
        /// (Z3a-4). Sampled once; the orbs then fly on their own.
        #[serde(default, skip_serializing_if = "Anchor::is_attacker")]
        anchor: Anchor,
        /// Moves the spawn point forward along the orb's direction and to the right of it.
        #[serde(default, skip_serializing_if = "Offset::is_zero")]
        offset: Offset,
        /// Orb asset; absent means the default orb.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<ContentId>,
        ticks: Expr,
        #[serde(default, skip_serializing_if = "crate::behaviour::is_false")]
        breakable: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        burst: Vec<Emitter>,
        /// Ticks after it is fired when the orb halts (0 to `ticks`): it stays where it is, and its
        /// lifetime runs on. It moves on the tick it is fired, so it moves `stop_after` times.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_after: Option<Expr>,
        /// Joins this orb to its neighbours in the ring it is fired in, once they are not flying
        /// toward a halt: the beams hurt the main character. Needs an enclosing `ring`, `spread` or
        /// `repeat` of at least 2 rounds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<Link>,
    },
    /// A zone that does not move (core Z1): it is placed at the `anchor`, moved by `offset`
    /// (forward along its `direction`, and to the right of it), warns for `wind_up_ticks`,
    /// hurts each target at most once for `active_ticks`, then ends. `ring`, `spread` and
    /// `rotate` turn its direction, and with it the offset, as they turn an orb's.
    Zone {
        shape: ZoneShape,
        #[serde(default, skip_serializing_if = "Anchor::is_attacker")]
        anchor: Anchor,
        #[serde(default)]
        offset: Offset,
        #[serde(default = "straight_on")]
        direction: Direction,
        /// 0 to `MAX_ZONE_TICKS`; 0 is a zone that hurts at once.
        wind_up_ticks: Expr,
        /// 1 to `MAX_ZONE_TICKS`. Required, unless the zone is `warn_only`, which refuses it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_ticks: Option<Expr>,
        /// A warning only (Z3a-4): it warns for `wind_up_ticks` (1 or more) and then ends. It
        /// never becomes active, never hurts and gives no invulnerability; its phase's `damage`,
        /// `type` and `breaks_charge` are not used for it. It counts in `MAX_ZONES_ALIVE`.
        #[serde(default, skip_serializing_if = "crate::behaviour::is_false")]
        warn_only: bool,
    },
}

/// The beams between the orbs of one ring (Z3a-3): between slot `j` and slot `j + 1` of the orbs
/// one `ring`, `spread` or `repeat` fires (the last to the first, unless there are only two), while
/// both are on the map. A beam is a rectangle `width` across that hurts the main character, each
/// beam at most once per `rehit_ticks`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// 1 to 64 pixels.
    pub width: Fx,
    /// The damage of a beam hit; absent means the orb's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_DAMAGE))]
    pub damage: Option<u32>,
    /// 1 to 600.
    #[schemars(range(min = 1, max = crate::validate::limits::MAX_PHASE_TICKS))]
    pub rehit_ticks: u32,
}

/// The direction of a zone that names none: at the target.
fn straight_on() -> Direction {
    Direction::Aimed {
        offset: Expr::Const(0),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Direction {
    /// At the target, turned by `offset`.
    Aimed { offset: Expr },
    /// A fixed angle: 0 points down the screen, and positive angles turn toward the right.
    Absolute { angle: Expr },
    /// Turned by `angle` from the parent's direction: straight down for an attack that a rule
    /// started, the orb's own direction for the zones of its `burst`.
    Relative { angle: Expr },
    /// Turned by `step` from the previous orb's or zone's direction; not affected by the turns of
    /// `rotate`, `ring` and `spread`.
    Sequential { step: Expr },
}

/// A fixed-point expression (ADR-036): a literal, an input, or an operator. No variables,
/// loops or recursion beyond the tree itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    /// The attack's argument at this index.
    Arg(u8),
    /// Index of the innermost `repeat`, `ring` or `spread`.
    LoopIndex,
    /// Ticks since the attack started: 0 on its first tick.
    PhaseTime,
    /// A draw from the run's SplitMix64 (ADR-033), from `lo` to `hi` inclusive; `lo` if the
    /// range is empty. Expressions are evaluated left to right, so the order of draws is fixed.
    Rand(Box<(Expr, Expr)>),
    Add(Box<(Expr, Expr)>),
    Sub(Box<(Expr, Expr)>),
    Mul(Box<(Expr, Expr)>),
    Div(Box<(Expr, Expr)>),
    Min(Box<(Expr, Expr)>),
    Max(Box<(Expr, Expr)>),
    /// `[value, lo, hi]`.
    Clamp(Box<(Expr, Expr, Expr)>),
    /// A literal; written as a plain integer.
    #[serde(untagged)]
    Const(
        #[schemars(range(min = -crate::validate::limits::MAX_NUMBER, max = crate::validate::limits::MAX_NUMBER))]
         i32,
    ),
}
