//! Demo binary that opens a Nebula Engine window with GPU-cleared background.
//!
//! Run with `cargo run -p nebula-demo` to see the window.

fn main() {
    env_logger::init();
    nebula_app::window::run();
}
