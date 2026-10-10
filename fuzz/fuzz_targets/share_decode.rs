#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    stella_rain_core_fuzz::share_decode(data);
});
