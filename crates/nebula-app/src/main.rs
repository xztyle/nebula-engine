//! The binary entry point for the Nebula Engine application.

#[allow(dead_code)]
mod platform;

fn main() {
    // Log preferred GPU backends.
    let _backends = platform::preferred_backends();

    // Resolve and create platform directories on startup.
    match platform::PlatformDirs::resolve_and_create() {
        Ok(dirs) => {
            println!("Nebula Engine App");
            println!("  config: {}", dirs.config_dir.display());
            println!("  data:   {}", dirs.data_dir.display());
            println!("  cache:  {}", dirs.cache_dir.display());
            println!("  logs:   {}", dirs.log_dir.display());
        }
        Err(e) => {
            eprintln!("Failed to initialize platform directories: {e}");
            std::process::exit(1);
        }
    }
}
