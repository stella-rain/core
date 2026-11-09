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
- Read core through its public API only, never through `src/` internals. Core never depends on net, except as a dev-dependency of one bench
  (`benches/rollback_session.rs`).
- `unsafe_code = "forbid"` (core can only `deny`, because `tests/alloc.rs` needs one `expect`).
  The four cast lints are `deny`, as in core.
- No third-party dependency without Kade: the only dependency is `stella-rain-core` by path.
  A new one is Kade's call, in the task that needs it, with the `deny.toml` list edited beside it.
- Sans-IO: ticks, not clocks; no sockets, threads, floats or hash-ordered collections. The root
  `clippy.toml` (`f32`, `f64`, `HashMap`, `HashSet`, clocks, threads, env reads) applies to
  `net/` unchanged, and `deny.toml` bans the netcode, async and socket crates by name.

## Workspace

- The root is a real package and the default member is core alone, so a command without `-p`
  (`cargo test`, `cargo clippy`, `cargo fmt --check`, `cargo bench`) does not touch `net/`.
  Run the net gate: `cargo fmt -p stella-rain-net --check`,
  `cargo clippy -p stella-rain-net --all-targets --locked -- -D warnings`,
  `cargo test -p stella-rain-net --locked`,
  `RUSTDOCFLAGS="-D warnings" cargo doc -p stella-rain-net --no-deps --locked`, and
  `cargo check -p stella-rain-net --locked --all-targets --target aarch64-apple-ios`.
  CI runs them in the `net` job of `rust.yml`.
- `fuzz/` and `xtask/` own their workspaces and locks. Profiles (overflow checks on) come from
  the root manifest; the package version and the release scripts read core's `[package]` only.
- Later files go in `net/src/` (`wire.rs`, `session.rs`, `predict.rs`) and `net/tests/`.
