# Server CLI Admin

## Problem

A dedicated game server must be administrable at runtime without restarting. Server operators need to inspect server state, manage players, force saves, and broadcast messages — all from the terminal that launched the server. Without a runtime admin interface:

- **Player management is impossible** — When a player is griefing, crashing the server to kick them disrupts every other connected player. The operator needs a `kick` command that removes a single player without downtime.
- **Graceful shutdown requires kill signals** — Without a `stop` command, the only way to shut down is `kill -9` or Ctrl+C, which may not flush world saves, may corrupt chunk data mid-write, and drops all players without a disconnect message.
- **Operational blindness** — Without a `status` command, the operator has no way to know how many players are connected, what the current tick rate is, or how long the server has been running. They must grep through log output or attach an external debugger.
- **No server-to-player communication** — Without a `say` command, the operator cannot announce scheduled maintenance, warn about upcoming restarts, or communicate with players from the server console.
- **Ban persistence is manual** — Without `ban` and `kick` commands, the operator must manually edit a ban list file and restart the server.

The admin interface reads commands from stdin in a dedicated thread (not async, because stdin blocking is fine for a single reader) and dispatches them to the server's ECS world and networking layer through a channel.

## Solution

### Command Definitions

```rust
use clap::{Parser, Subcommand};

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Start accepting player connections (if paused)
    Start,

    /// Gracefully shut down the server
    Stop,

    /// Show server status: player count, uptime, tick rate, memory
    Status,

    /// Kick a player by name or connection ID
    Kick {
        /// Player name or numeric connection ID
        player: String,
    },

    /// Ban a player by name (also kicks if online)
    Ban {
        /// Player name to ban
        player: String,

        /// Optional reason for the ban
        #[arg(long)]
        reason: Option<String>,
    },

    /// Unban a previously banned player
    Unban {
        /// Player name to unban
        player: String,
    },

    /// Force an immediate world save
    Save,

    /// Broadcast a chat message to all connected players
    Say {
        /// Message to broadcast
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// List all connected players
    List,

    /// Show help for available commands
    Help,
}
```

Note: `clap` is not used to parse these commands from `argv` — it is used to parse them from the stdin line. Each input line is split into tokens and passed through `clap`'s `try_parse_from` method, which gives us automatic help text, argument validation, and error messages for free.

### Stdin Reader Thread

The admin interface runs on a dedicated OS thread (not a tokio task) because `std::io::stdin().read_line()` is a blocking syscall that should not block the async runtime's thread pool:

```rust
use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

pub fn spawn_stdin_reader(cmd_tx: mpsc::UnboundedSender<AdminCommand>) {
    std::thread::Builder::new()
        .name("admin-cli".to_string())
        .spawn(move || {
            let stdin = io::stdin();
            let reader = stdin.lock();

            // Print prompt
            print!("> ");
            io::stdout().flush().ok();

            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l.trim().to_string(),
                    Err(_) => break, // stdin closed
                };

                if line.is_empty() {
                    print!("> ");
                    io::stdout().flush().ok();
                    continue;
                }

                // Prepend a dummy program name for clap's argv parsing
                let mut argv = vec!["server".to_string()];
                argv.extend(line.split_whitespace().map(String::from));

                match parse_admin_command(&argv) {
                    Ok(cmd) => {
                        if cmd_tx.send(cmd).is_err() {
                            // Receiver dropped, server is shutting down
                            break;
                        }
                    }
                    Err(e) => {
                        // clap already prints the error/help to stderr
                        eprintln!("{e}");
                    }
                }

                print!("> ");
                io::stdout().flush().ok();
            }

            tracing::debug!("Admin CLI thread exiting");
        })
        .expect("Failed to spawn admin CLI thread");
}

/// Parse a command line into an AdminCommand using clap.
fn parse_admin_command(argv: &[String]) -> Result<AdminCommand, String> {
    #[derive(Parser)]
    #[command(name = "server", no_binary_name = false, disable_help_flag = false)]
    struct Cli {
        #[command(subcommand)]
        command: AdminCommand,
    }

    Cli::try_parse_from(argv)
        .map(|cli| cli.command)
        .map_err(|e| e.to_string())
}
```

### Command Dispatch

Commands are consumed on the server's main async task via a `mpsc::UnboundedReceiver`. Each command is dispatched to the appropriate handler:

```rust
use tokio::sync::{mpsc, watch};

pub async fn process_admin_command(
    cmd: AdminCommand,
    server_state: &mut ServerState,
    shutdown_tx: &watch::Sender<bool>,
) {
    match cmd {
        AdminCommand::Start => {
            server_state.accepting_connections = true;
            tracing::info!("Server is now accepting connections");
            println!("Server is now accepting connections.");
        }

        AdminCommand::Stop => {
            tracing::info!("Admin issued stop command");
            println!("Initiating graceful shutdown...");
            // Trigger world save before shutdown
            server_state.force_save().await;
            let _ = shutdown_tx.send(true);
        }

        AdminCommand::Status => {
            let status = server_state.get_status();
            println!("=== Server Status ===");
            println!("  Uptime:      {}", format_duration(status.uptime));
            println!("  Players:     {}/{}", status.player_count, status.max_players);
            println!("  Tick rate:   {:.1} tps", status.current_tick_rate);
            println!("  Chunks:      {}", status.loaded_chunks);
            println!("  Memory:      {:.1} MB", status.memory_mb);
            println!("=====================");
        }

        AdminCommand::Kick { player } => {
            match server_state.kick_player(&player).await {
                Ok(name) => {
                    tracing::info!("Kicked player: {name}");
                    println!("Kicked {name}.");
                }
                Err(e) => println!("Failed to kick: {e}"),
            }
        }

        AdminCommand::Ban { player, reason } => {
            let reason_str = reason.as_deref().unwrap_or("No reason given");
            match server_state.ban_player(&player, reason_str).await {
                Ok(name) => {
                    tracing::info!(reason = reason_str, "Banned player: {name}");
                    println!("Banned {name} ({reason_str}).");
                }
                Err(e) => println!("Failed to ban: {e}"),
            }
        }

        AdminCommand::Unban { player } => {
            match server_state.unban_player(&player) {
                Ok(()) => {
                    tracing::info!("Unbanned player: {player}");
                    println!("Unbanned {player}.");
                }
                Err(e) => println!("Failed to unban: {e}"),
            }
        }

        AdminCommand::Save => {
            println!("Forcing world save...");
            server_state.force_save().await;
            println!("World saved.");
        }

        AdminCommand::Say { message } => {
            let text = message.join(" ");
            server_state.broadcast_chat(&format!("[Server] {text}")).await;
            tracing::info!("Broadcast: {text}");
            println!("[Server] {text}");
        }

        AdminCommand::List => {
            let players = server_state.list_players();
            if players.is_empty() {
                println!("No players connected.");
            } else {
                println!("Connected players ({}):", players.len());
                for p in &players {
                    println!("  - {} (id: {}, ping: {}ms)", p.name, p.id, p.ping_ms);
                }
            }
        }

        AdminCommand::Help => {
            println!("Available commands:");
            println!("  start          - Accept connections");
            println!("  stop           - Graceful shutdown");
            println!("  status         - Server status");
            println!("  kick <player>  - Kick a player");
            println!("  ban <player>   - Ban a player");
            println!("  unban <player> - Unban a player");
            println!("  save           - Force world save");
            println!("  say <message>  - Broadcast message");
            println!("  list           - List connected players");
            println!("  help           - Show this help");
        }
    }
}
```

### Ban List Persistence

The ban list is stored in a `bans.json` file alongside the server config. It is loaded on startup and written after every `ban` or `unban` command:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BanList {
    /// Map of player name (lowercase) -> ban entry
    pub entries: HashMap<String, BanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanEntry {
    pub player_name: String,
    pub reason: String,
    pub banned_at: u64, // Unix timestamp
}
```

### Integration with Server Tick Loop

The admin command receiver is polled non-blockingly at the start of each server tick, before processing network messages. This ensures commands are handled promptly without disrupting the tick timing:

```rust
// Inside the tick loop (story 03)
while let Ok(cmd) = admin_rx.try_recv() {
    process_admin_command(cmd, &mut server_state, &shutdown_tx).await;
}
```

## Outcome

A `cli_admin.rs` module in `crates/nebula-server/src/` that provides a runtime admin interface. The server reads commands from stdin on a dedicated thread, parses them with `clap`, and dispatches them to the server state via an `mpsc` channel. Supported commands: `start`, `stop`, `status`, `kick`, `ban`, `unban`, `save`, `say`, `list`, `help`. The ban list persists to disk. Commands are processed at the top of each server tick. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

An interactive CLI accepts admin commands: `/list` shows connected players, `/kick <name>` disconnects a player, `/save` triggers a world save, `/stop` shuts down gracefully.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `clap` | `4` (features: `derive`) | Parse admin commands from stdin input using subcommand derive macros |
| `tokio` | `1.49` (features: `sync`) | `mpsc` channel for command dispatch, `watch` channel for shutdown signal |
| `tracing` | `0.1` | Structured logging of admin actions |
| `serde` | `1.0` (features: `derive`) | Serialize/deserialize the ban list |
| `serde_json` | `1.0` | Ban list file format (JSON for easy hand-editing) |

## Unit Tests

- **`test_start_command_parses`** — Parse the input line `"start"` through `parse_admin_command`. Assert the result is `Ok(AdminCommand::Start)`. This validates that the simplest command parses correctly.

- **`test_stop_command_initiates_shutdown`** — Create a `watch::channel` for shutdown. Call `process_admin_command(AdminCommand::Stop, ...)` with the sender. Assert that the watch receiver's value is `true` after the command completes. This validates that `stop` triggers the shutdown signal.

- **`test_status_reports_correct_info`** — Create a `ServerState` with 5 connected players, a known uptime of 3600 seconds, and a tick rate of 60.0. Call `server_state.get_status()` and assert `player_count == 5`, `uptime.as_secs() == 3600`, and `current_tick_rate` is approximately 60.0. This validates the status data path.

- **`test_kick_removes_player`** — Create a `ServerState` with a player named `"Alice"` connected. Call `server_state.kick_player("Alice")`. Assert it returns `Ok("Alice")`. Assert `server_state.list_players()` no longer contains `"Alice"`. This validates the kick flow.

- **`test_kick_nonexistent_player_returns_error`** — Call `server_state.kick_player("nobody")` on a server with no player by that name. Assert the result is `Err` with a message containing `"not found"`.

- **`test_ban_adds_to_ban_list`** — Call `server_state.ban_player("Bob", "griefing")`. Assert the ban list contains an entry for `"bob"` (lowercase) with reason `"griefing"`. If Bob was connected, assert he is also kicked.

- **`test_command_parse_kick_with_argument`** — Parse the input line `"kick Alice"` through `parse_admin_command`. Assert the result is `Ok(AdminCommand::Kick { player: "Alice".to_string() })`.

- **`test_command_parse_say_with_multiple_words`** — Parse the input line `"say Hello everyone, server restarting soon"`. Assert the result is `Ok(AdminCommand::Say { message })` where `message.join(" ")` equals `"Hello everyone, server restarting soon"`.

- **`test_command_parse_invalid_returns_error`** — Parse the input line `"frobnicate"` through `parse_admin_command`. Assert the result is `Err` (unknown subcommand).

- **`test_ban_list_roundtrip`** — Create a `BanList` with two entries, serialize to JSON, deserialize back, and assert equality. This validates ban list persistence.
