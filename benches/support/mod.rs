//! Shared by the bench targets; not a target itself (Cargo discovers only `benches/*.rs`).

/// The teardown of every bench: the result goes through `black_box` and is forgotten.
///
/// In iai-callgrind 0.16 the teardown runs inside the counted wrapper, so a drop here would count
/// `drop_glue` and the allocator's consolidation, which depend on the heap layout and are not the
/// operation a bench names. The value is leaked instead; each bench is its own process, so the
/// leak ends with it.
pub fn forget_measured<T>(value: T) {
    std::mem::forget(std::hint::black_box(value));
}
