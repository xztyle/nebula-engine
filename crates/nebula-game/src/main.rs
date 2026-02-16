//! Elite Dangerous Lite — clean game entry point for the Nebula Engine.
//!
//! Opens a window with a real-scale Earth planet visible from orbit,
//! initializes the wgpu renderer, runs the AI Debug API on port 9999,
//! and provides a free-fly camera for exploring the planet.
//!
//! Run with: `cargo run -p nebula-game`

mod planet;

use clap::Parser;
use nebula_config::Config;
use tracing::info;

/// CLI arguments for the game binary.
#[derive(Parser, Debug)]
#[command(name = "nebula-game", about = "Elite Dangerous Lite — Nebula Engine")]
struct GameArgs {
    /// Window width in pixels.
    #[arg(long, default_value_t = 1600)]
    width: u32,

    /// Window height in pixels.
    #[arg(long, default_value_t = 900)]
    height: u32,

    /// Window title override.
    #[arg(long)]
    title: Option<String>,
}

fn main() {
    let args = GameArgs::parse();

    // Start with default config, then apply CLI overrides.
    let mut config = Config::default();

    config.window.width = args.width;
    config.window.height = args.height;
    config.window.title = args
        .title
        .unwrap_or_else(|| "Nebula Engine — Elite Dangerous Lite".to_string());

    // Configure Earth-scale planet and orbital camera.
    config.planet = planet::earth_config();

    // Initialize structured logging.
    nebula_log::init_logging(None, cfg!(debug_assertions), Some(&config));

    info!("Nebula Engine - Elite Dangerous Lite");
    info!(
        "Window: {}x{} | Title: {}",
        config.window.width, config.window.height, config.window.title
    );
    info!(
        "Planet: radius={:.0}km, altitude={:.0}km",
        config.planet.radius_m / 1000.0,
        config.planet.start_altitude_m / 1000.0,
    );

    // Run the engine: opens window, initializes wgpu, starts debug API on :9999,
    // and enters the fixed-timestep game loop.
    nebula_app::window::run_with_config_and_input(config, |dt, keyboard, mouse| {
        game_update(dt, keyboard, mouse);
    });
}

/// Per-tick game simulation update.
///
/// Called at a fixed 60 Hz rate with keyboard and mouse state.
/// This is where ship physics, planet systems, and game logic will live.
fn game_update(
    _dt: f64,
    _keyboard: &nebula_input::KeyboardState,
    _mouse: &nebula_input::MouseState,
) {
    // TODO(story-03): Ship 6DOF physics from keyboard/mouse input
    // TODO(story-05): Update HUD state
}
