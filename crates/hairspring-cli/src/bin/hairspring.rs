fn main() -> std::process::ExitCode {
    // SAFETY: nothing has started a thread or used SQLite yet; this is the first statement of the process.
    unsafe { hairspring_store::disable_sqlite_memory_statistics() };
    hairspring_cli::main_with(hairspring_engine::demos::default_registry())
}
