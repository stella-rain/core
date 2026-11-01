//! The rollback measurement harness (throwaway branch, never merged): prints the CSV of
//! `spikes/README.md` on stdout and the build on stderr.
//! `spike_rollback [--device NAME] [--samples N] [--warm-up N]`

#[path = "../../spikes/harness.rs"]
mod harness;

fn main() {
    let mut device = String::from("desktop");
    let mut config = harness::Config {
        samples: 2000,
        warm_up: 100,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_default();
        match flag.as_str() {
            "--device" => device = value,
            "--samples" => config.samples = value.parse().expect("--samples N"),
            "--warm-up" => config.warm_up = value.parse().expect("--warm-up N"),
            _ => panic!("unknown flag {flag}"),
        }
    }
    eprintln!(
        "# build: sha={} rustc={} release={} device={device} samples={} warm_up={}",
        option_env!("SPIKE_SHA").unwrap_or("unknown"),
        option_env!("SPIKE_RUSTC").unwrap_or("unknown"),
        !cfg!(debug_assertions),
        config.samples,
        config.warm_up
    );
    let mut out = std::io::stdout().lock();
    harness::run(&mut out, config, &device).expect("writes the CSV");
}
