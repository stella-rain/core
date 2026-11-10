---
paths:
  - "net/**"
---

# The net crate

`net/` is the package `stella-rain-net`: sans-IO rollback netcode (input exchange, prediction and
re-simulation) over the public API of `stella-rain-core`. It ships in core's own release; its
version stays `0.0.0` and `publish = false`.

## Rules

- Public API: nothing is `pub` without a doc comment and a test. `missing_docs` is `deny`.
- Read core through its public API only, never through `src/` internals. Core never depends on net, except as a
  dev-dependency of the `rollback_session` bench (`benches/rollback_session.rs`) and its pins in `tests/bench_fixture_pins.rs`.
- The wire (`wire.rs`): tags are never reused (the retired protocol 1 hello keeps `0x01`); a retired hello layout stays
  decodable so its peer is refused, not dropped as garbage; vectors (`net/tests/vectors.rs`) are written from the layout, never by
  running the encoder; a layout change raises `PROTOCOL_VERSION`.
- The rollback `Session` steps a tick in the recorder's order (`step_tick`): save the state, make the leaves stamped for the
  tick, make the actions in slot order (a refusal is ignored), then step on inputs with the action cleared. `receive_inputs`
  checks a whole frame before it queues any (slot, a stamped leave, `TooFarAhead` past `tick + 2 * delay + window + 1`,
  `Conflict` with a held input); `wire.rs` refuses a stick axis outside -127 to 127.
- `CloneRing` saves and restores in place: a slot is written by `Clone::clone_from`, so `Engine::clone_from` reuses a held
  engine's buffers and a full ring drops no engine (bench `clone_ring_resave`, `benches-fuzz.md`).
- A change under `net/` runs the short fuzz (`rust.yml` counts it as parsing); `net_decode` takes its seeds from `write_net_seeds`
  (`benches-fuzz.md`).
- `unsafe_code = "forbid"` (core can only `deny`, because `tests/alloc.rs` needs one `expect`).
  The four cast lints are `deny`, as in core.
- No third-party dependency without Kade: the only dependency is `stella-rain-core` by path.
  A new one is Kade's call, in the task that needs it, with the `deny.toml` list edited beside it.
- Sans-IO: ticks, not clocks; no sockets, threads, floats or hash-ordered collections. The root
  `clippy.toml` (`f32`, `f64`, `HashMap`, `HashSet`, clocks, threads, env reads) applies to
  `net/` unchanged, and `deny.toml` bans the netcode, async and socket crates by name.

## Workspace

- The root is a real package and the default member is core alone, so a command without `-p` (`cargo test`, `cargo clippy`,
  `cargo fmt --check`, `cargo bench`) runs none of net's own checks, though `cargo test`, `cargo clippy --all-targets` and
  `cargo bench` do compile it, as a dev-dependency (`Cargo.toml`).
  Run the net gate: `cargo fmt -p stella-rain-net --check`,
  `cargo clippy -p stella-rain-net --all-targets --locked -- -D warnings`,
  `cargo test -p stella-rain-net --locked`,
  `RUSTDOCFLAGS="-D warnings" cargo doc -p stella-rain-net --no-deps --locked`, and
  `cargo check -p stella-rain-net --locked --all-targets --target aarch64-apple-ios`.
  CI runs them in the `net` job of `rust.yml`.
- `fuzz/` and `xtask/` own their workspaces and locks. Profiles (overflow checks on) come from
  the root manifest; the package version and the release scripts read core's `[package]` only.
- `net/src/`: `wire.rs`, `session.rs` (with `session/party.rs`, `report.rs`), `predict.rs`, `save.rs`, `transport.rs`; tests in `net/tests/`.
