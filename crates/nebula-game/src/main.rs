//! Elite Dangerous Lite — clean game entry point for the Nebula Engine.
//!
//! Opens a window with a real-scale Earth planet visible from orbit,
//! initializes the wgpu renderer, runs the AI Debug API on port 9999,
//! and provides a ship with 6DOF Newtonian flight for exploring the planet.
//!
//! Run with: `cargo run -p nebula-game`

mod hud;
mod planet;
mod ship;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use clap::Parser;
use nebula_config::Config;
use tracing::info;
use winit::keyboard::{KeyCode, PhysicalKey};

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

    // Create ship at ISS orbital altitude.
    let ship_config = ship::ShipConfig::default();
    let mut ship_state =
        ship::ShipState::at_orbit(config.planet.radius_m, config.planet.start_altitude_m);

    info!(
        "Ship: mass={:.0}kg, thrust={:.0}N, pos=({:.0}, {:.0}, {:.0})",
        ship_config.mass,
        ship_config.max_thrust,
        ship_state.position.x,
        ship_state.position.y,
        ship_state.position.z,
    );

    let planet_radius_m = config.planet.radius_m;
    let atmosphere_altitude_m = config.planet.atmosphere_altitude_m;

    // Shared HUD state between the update and title callbacks.
    let hud_state = Rc::new(RefCell::new(hud::HudState::default()));
    let hud_for_title = Rc::clone(&hud_state);

    // Shared clear color between the update callback and the clear color callback.
    // Default to deep space black.
    let clear_color = Rc::new(Cell::new([0.02_f64, 0.02, 0.08]));
    let clear_color_for_render = Rc::clone(&clear_color);

    // Run the engine with custom input, HUD title, and dynamic clear color.
    nebula_app::window::run_with_config_input_title_and_clear(
        config,
        move |dt, keyboard, mouse, camera| {
            // Detect thrust and boost state for HUD throttle display.
            let is_thrusting = keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyW))
                || keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyS))
                || keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyA))
                || keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyD))
                || keyboard.is_pressed(PhysicalKey::Code(KeyCode::Space))
                || keyboard.is_pressed(PhysicalKey::Code(KeyCode::ShiftLeft));
            let is_boosting = keyboard.is_pressed(PhysicalKey::Code(KeyCode::ControlLeft));

            ship::update_ship(&mut ship_state, &ship_config, dt, keyboard, mouse);
            ship::sync_camera_to_ship(camera, &ship_state, is_boosting);

            // Atmospheric clear color: lerp from space black to sky blue based on altitude.
            let altitude = (ship_state.position.length() - planet_radius_m).max(0.0);
            let atmo_t = if atmosphere_altitude_m > 0.0 {
                (1.0 - altitude / atmosphere_altitude_m).clamp(0.0, 1.0)
            } else {
                0.0
            };
            // Space color -> sky blue (0.4, 0.6, 0.9)
            let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
            clear_color.set([
                lerp(0.02, 0.4, atmo_t),
                lerp(0.02, 0.6, atmo_t),
                lerp(0.08, 0.9, atmo_t),
            ]);

            // Reduce far plane in atmosphere for less deep-space visibility.
            let base_far = planet_radius_m as f32 * 4.0;
            let atmo_far = 200_000.0_f32; // 200 km visibility in thick atmosphere
            camera.far = base_far + (atmo_far - base_far) * atmo_t as f32;

            // Update HUD telemetry.
            hud::update_hud(
                &mut hud_state.borrow_mut(),
                &ship_state,
                planet_radius_m,
                is_thrusting,
                is_boosting,
            );
        },
        move || hud::format_hud(&hud_for_title.borrow()),
        move |_tick| {
            let c = clear_color_for_render.get();
            [c[0], c[1], c[2], 1.0]
        },
    );
}
