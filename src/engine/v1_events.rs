//! Turns the events of the frozen rules of `sim_version` 1 into the live `DomainEvent`
//! (ADR-REPL-2 points 1 to 4). A change to the live event edits this file
//! and never `sim_v1`. The match is exhaustive and moves every field by name; the fields the
//! live type gained for players are `None`, since version 1 names no player. It also converts
//! the constellation view the frozen rules build (`sim_v1::views`, a copy of the live one) to the
//! live type. The offer views and events of version 1 have no live counterpart (ADR-REPL-5).

use crate::event::{DomainEvent, HitKind, HitSource};
use crate::sim_v1::event as v1;
use crate::sim_v1::views as views1;
use crate::snapshot::ConstellationView;

impl From<views1::ConstellationView> for ConstellationView {
    fn from(view: views1::ConstellationView) -> Self {
        let views1::ConstellationView {
            id,
            name,
            held,
            tier,
            needed,
        } = view;
        ConstellationView {
            id,
            name,
            held,
            tier,
            needed,
        }
    }
}

impl From<v1::HitKind> for HitKind {
    fn from(kind: v1::HitKind) -> Self {
        match kind {
            v1::HitKind::Attack => HitKind::Attack,
            v1::HitKind::Zone => HitKind::Zone,
            v1::HitKind::Link => HitKind::Link,
        }
    }
}

impl From<v1::HitSource> for HitSource {
    fn from(source: v1::HitSource) -> Self {
        let v1::HitSource {
            kind,
            owner,
            attack,
        } = source;
        HitSource {
            kind: kind.into(),
            owner,
            attack,
        }
    }
}

/// The live event of a frozen one, or `None` for the eight offer events the live rules no
/// longer have: the four level-offer ones (a stat offer opening, an expertise offer opening, an
/// option taken, a candidate replaced) and the star offer's (opened, skipped, a reward banished,
/// a candidate replaced; ADR-REPL-5, ADR-SIM-7). The match is exhaustive, with no `_` arm.
pub(super) fn adapt(event: v1::DomainEvent) -> Option<DomainEvent> {
    // The four player fields (`player`, `buffed_by`, `source`, `killer`) are `None` below:
    // version 1 names no player.
    let adapted = match event {
        v1::DomainEvent::Hit {
            source,
            target,
            target_kind,
            damage,
            damage_type,
            match_up,
            rune,
            crit,
            echo,
        } => DomainEvent::Hit {
            source,
            target,
            target_kind,
            damage,
            damage_type,
            match_up,
            rune,
            crit,
            echo,
            repeat: false,
            player: None,
            buffed_by: None,
        },
        v1::DomainEvent::KillBurst { at, radius, depth } => {
            DomainEvent::KillBurst { at, radius, depth }
        }
        v1::DomainEvent::PlayerHit { hp_left, source } => DomainEvent::PlayerHit {
            hp_left,
            source: source.into(),
        },
        v1::DomainEvent::Healed { target, amount } => DomainEvent::Healed {
            target,
            amount,
            source: None,
        },
        v1::DomainEvent::StatusApplied {
            target,
            status,
            stacks,
            duration_ticks,
        } => DomainEvent::StatusApplied {
            target,
            status,
            stacks,
            duration_ticks,
            source: None,
        },
        v1::DomainEvent::StunEnded { target } => DomainEvent::StunEnded { target },
        v1::DomainEvent::Evaded => DomainEvent::Evaded,
        v1::DomainEvent::StageSkillUsed { skill, level } => {
            DomainEvent::StageSkillUsed { skill, level }
        }
        v1::DomainEvent::ComboStep { press, depth } => DomainEvent::ComboStep { press, depth },
        v1::DomainEvent::ComboReset => DomainEvent::ComboReset,
        v1::DomainEvent::ChargeReleased { level, perfect } => {
            DomainEvent::ChargeReleased { level, perfect }
        }
        v1::DomainEvent::ChargeBroken => DomainEvent::ChargeBroken,
        v1::DomainEvent::HoldReleased { ticks } => DomainEvent::HoldReleased { ticks },
        v1::DomainEvent::HoldBroken => DomainEvent::HoldBroken,
        v1::DomainEvent::RuleFired {
            agent,
            rule,
            target,
        } => DomainEvent::RuleFired {
            agent,
            rule,
            target,
        },
        v1::DomainEvent::PhaseChanged { phase } => DomainEvent::PhaseChanged { phase },
        v1::DomainEvent::Telegraph { source, ticks } => DomainEvent::Telegraph { source, ticks },
        v1::DomainEvent::PartBroken { part } => DomainEvent::PartBroken { part },
        v1::DomainEvent::Died {
            entity,
            kind,
            class,
            elite,
        } => DomainEvent::Died {
            entity,
            kind,
            class,
            elite,
            killer: None,
        },
        v1::DomainEvent::StageCleared => DomainEvent::StageCleared,
        v1::DomainEvent::StageFailed => DomainEvent::StageFailed,
        v1::DomainEvent::LeveledUp { level } => DomainEvent::LeveledUp { level },
        v1::DomainEvent::StatRaised { stat, spent } => DomainEvent::StatRaised { stat, spent },
        v1::DomainEvent::StatOfferOpened { .. } => return None,
        v1::DomainEvent::StarDropped { star, rarity } => DomainEvent::StarDropped { star, rarity },
        v1::DomainEvent::StarTaken { star, rarity } => DomainEvent::StarTaken { star, rarity },
        v1::DomainEvent::HealDropped { pickup } => DomainEvent::HealDropped { pickup },
        v1::DomainEvent::ElitesCalled { elites } => DomainEvent::ElitesCalled { elites },
        v1::DomainEvent::EliteSpawned { elite, asset, kind } => {
            DomainEvent::EliteSpawned { elite, asset, kind }
        }
        v1::DomainEvent::EnemyRelocated { entity } => DomainEvent::EnemyRelocated { entity },
        v1::DomainEvent::EliteLeft { elite } => DomainEvent::EliteLeft { elite },
        v1::DomainEvent::ShrineActivated { elite } => DomainEvent::ShrineActivated { elite },
        v1::DomainEvent::MimicRevealed { elite } => DomainEvent::MimicRevealed { elite },
        v1::DomainEvent::OrbBroken { owner, at } => DomainEvent::OrbBroken { owner, at },
        v1::DomainEvent::RewardTaken {
            reward,
            taken,
            rune,
            star,
        } => DomainEvent::RewardTaken {
            reward,
            taken,
            rune,
            star,
        },
        v1::DomainEvent::Constellation { set, tier } => DomainEvent::Constellation { set, tier },
        v1::DomainEvent::OfferOpened { .. }
        | v1::DomainEvent::OfferSkipped
        | v1::DomainEvent::RewardBanished { .. }
        | v1::DomainEvent::CandidateReplaced { .. }
        | v1::DomainEvent::ExpertiseOfferOpened { .. }
        | v1::DomainEvent::ExpertiseOptionTaken { .. }
        | v1::DomainEvent::ExpertiseCandidateReplaced { .. } => return None,
    };
    Some(adapted)
}
