fn main() -> std::process::ExitCode {
    // SAFETY: nothing has started a thread or used SQLite yet; this is the first statement of the process.
    let statistics_off = unsafe { hairspring_store::disable_sqlite_memory_statistics() };
    if !statistics_off {
        eprintln!(
            "hairspring: sqlite memory statistics could not be disabled; parallel reads may be slower"
        );
    }
    hairspring_cli::main_with(hairspring_engine::demos::default_registry())
}
