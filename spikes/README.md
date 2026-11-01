# Rollback measurement harness (throwaway branch, never merged)

Times cloning and restoring the engine plus W re-simulated ticks. `Engine::step` takes one
player's input, so players 2 and 3 are a proxy: that many independent engines cloned, restored
and stepped in lockstep inside one measured interval (an upper bound; a shared world repeats
the crowd once, not N times). The code is `spikes/harness.rs`; the binary is
`src/bin/spike_rollback.rs`; the tests are `tests/spike_rollback.rs`.

## CSV

`device,scenario,players_proxy,window,op,p50,p95,p99,max,samples`, times in microseconds.
Scenarios: `worst_tick`, `worst_minute_peak`, `time_limit_tick` (fixtures of `bench_fixtures`),
each at 1, 2 and 3 engines, with these ops (window W is in ticks):

| op | window | what is timed |
|---|---|---|
| `clone` | 0 | clone the engines and drop the copies |
| `restore` | 0 | replace engines that stepped one tick with saved copies (their drop included) |
| `step` | 1 | one tick of each engine |
| `resim` | 1, 2, 4, 6, 8 | restore from saved copies, then W ticks of each engine |

That is 3 x 3 x 8 = 72 rows. Two more rows, scenario `time_limit_run_10800`, 1 player,
window 0: `savepoint_save` (`RunRecorder::save`) and `savepoint_restore`
(`resume_from_savepoint`, with the replay prefix hashed) of the bundled `time_limit` run at
tick 10,800 with the fixtures' enemy damage of 100 (a light schedule, not the dense minute).
74 rows in all. Setup (copies, the dirty engines) is outside the interval. The fixtures'
own input is stepped each tick; the dense minute taps the main weapon every other tick and
takes the first option of an open offer. Default 2,000 samples per row after 100 dropped
(`--samples N`, `--warm-up N`); use 500 on the phone.

## Desktop

```bash
export SPIKE_SHA=$(git rev-parse HEAD) SPIKE_RUSTC="$(rustc -V)"
cargo build --release --features bench --bin spike_rollback
./target/release/spike_rollback --device "desktop-<model, no commas>" > spikes/results/desktop.csv
```

## Phone (Lenovo Tab P11, arm64 Android)

Needs `rustup target add aarch64-linux-android` (in `rust-toolchain.toml`) and an NDK. Close
other apps, screen on, battery above 50%, airplane mode; note how warm the tablet is.

```bash
NDK=$HOME/Library/Android/sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64/bin
export SPIKE_SHA=$(git rev-parse HEAD) SPIKE_RUSTC="$(rustc -V)"
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android30-clang \
CC_aarch64_linux_android=$NDK/aarch64-linux-android30-clang AR_aarch64_linux_android=$NDK/llvm-ar \
  cargo build --release --features bench --bin spike_rollback --target aarch64-linux-android
adb push target/aarch64-linux-android/release/spike_rollback /data/local/tmp/
adb shell chmod +x /data/local/tmp/spike_rollback
adb shell /data/local/tmp/spike_rollback --device lenovo-p11 --samples 500 > spikes/results/phone.csv
git add spikes/results/phone.csv
```

(The NDK `prebuilt` folder is `darwin-x86_64` on a Mac, also for Apple silicon.) The build
and settings print on stderr. Commit `spikes/results/phone.csv` to this branch and push, or
hand the file over.
