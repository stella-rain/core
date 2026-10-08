//! ADR-014 week-one spike: the smallest simulation that exercises fixed-point movement, a
//! seeded PRNG, ordered entity storage, a budget cap and the state hash. It is a test bed
//! for the "same hash on the phone as on x86_64" check, not a frozen `rules_vN`.

use crate::fixed::Fx;
use crate::hash::StateHasher;
use crate::rng::SplitMix64;

const FIELD_W: Fx = Fx::px(360);
const FIELD_H: Fx = Fx::px(640);
const PLAYER_SPEED: Fx = Fx::px(3);
const FOCUS_SPEED: Fx = Fx::px(1);
const PLAYER_RADIUS: i32 = Fx::px(3).0;
const SPAWN_EVERY: u32 = 10;
/// Bullets alive at once; spawns past the cap are dropped (ADR-020's overflow rule).
const MAX_BULLETS: u32 = 4000;

/// One input record per tick: the only external influence (ADR-004).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    /// -1, 0 or 1 on each axis.
    pub dx: i8,
    pub dy: i8,
    pub focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bullet {
    x: Fx,
    y: Fx,
    vx: Fx,
    vy: Fx,
}

#[derive(Clone, Debug)]
pub struct Spike {
    tick: u32,
    rng: SplitMix64,
    x: Fx,
    y: Fx,
    hits: u32,
    bullets: Vec<Bullet>,
}

impl Spike {
    pub fn new(seed: u64) -> Self {
        Self {
            tick: 0,
            rng: SplitMix64::new(seed),
            x: Fx(FIELD_W.0 / 2),
            y: Fx(FIELD_H.0 - Fx::px(64).0),
            hits: 0,
            bullets: Vec::new(),
        }
    }

    pub fn step(&mut self, input: Input) {
        self.tick += 1;

        let speed = if input.focus {
            FOCUS_SPEED
        } else {
            PLAYER_SPEED
        };
        let step = |d: i8| Fx(i32::from(d.signum()) * speed.0);
        self.x = self.x.saturating_add(step(input.dx)).clamp(Fx(0), FIELD_W);
        self.y = self.y.saturating_add(step(input.dy)).clamp(Fx(0), FIELD_H);

        if self.tick % SPAWN_EVERY == 0 && self.bullet_count() < MAX_BULLETS {
            let x = Fx(self.rng.below(FIELD_W.0 as u32) as i32);
            let vx = Fx::px(self.rng.below(3) as i32 - 1);
            let vy = Fx::px(2 + self.rng.below(3) as i32);
            self.bullets.push(Bullet {
                x,
                y: Fx(0),
                vx,
                vy,
            });
        }

        for b in &mut self.bullets {
            b.x = b.x.saturating_add(b.vx);
            b.y = b.y.saturating_add(b.vy);
        }
        let (px, py) = (self.x.0, self.y.0);
        let r2 = i64::from(PLAYER_RADIUS) * i64::from(PLAYER_RADIUS);
        let before = self.bullets.len();
        self.bullets.retain(|b| {
            let dx = i64::from(b.x.0) - i64::from(px);
            let dy = i64::from(b.y.0) - i64::from(py);
            dx * dx + dy * dy > r2
        });
        let hit = u32::try_from(before - self.bullets.len()).expect("hits fit in u32");
        self.hits = self.hits.saturating_add(hit);
        self.bullets
            .retain(|b| (0..=FIELD_W.0).contains(&b.x.0) && (0..=FIELD_H.0).contains(&b.y.0));
    }

    pub fn bullet_count(&self) -> u32 {
        u32::try_from(self.bullets.len()).expect("bullet count fits in u32")
    }

    /// FNV-1a 64 over the whole state, field by field, at fixed widths (ADR-019).
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u32(self.tick);
        h.write_u64(self.rng.state());
        h.write_i32(self.x.0);
        h.write_i32(self.y.0);
        h.write_u32(self.hits);
        h.write_u32(self.bullet_count());
        for b in &self.bullets {
            h.write_i32(b.x.0);
            h.write_i32(b.y.0);
            h.write_i32(b.vx.0);
            h.write_i32(b.vy.0);
        }
        h.finish()
    }
}
