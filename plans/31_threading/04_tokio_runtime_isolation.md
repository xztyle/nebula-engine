# Tokio Runtime Isolation

## Problem

The game engine has two fundamentally different kinds of concurrent work:

1. **CPU-bound computation** -- Meshing, terrain generation, physics prep, LOD calculation. These tasks need all available CPU cycles and benefit from work-stealing parallelism. They run on the rayon thread pool (stories 01 and 05).

2. **I/O-bound async work** -- TCP networking (connecting to servers, receiving multiplayer state), async file I/O (loading chunk data from disk, writing save files), and async asset loading (streaming textures). These tasks spend most of their time waiting for the OS or network and need an async runtime to avoid wasting threads on blocking waits.

If tokio and rayon share the same threads, two catastrophic interactions emerge:

- **CPU starvation of I/O** -- A burst of meshing work saturates all cores, and network heartbeats stop being sent, causing the server to disconnect the client for timeout.
- **I/O blocking CPU work** -- A tokio task accidentally runs a synchronous file read on a rayon thread (via `block_on`), stalling the work-stealing pool and halving throughput.

The engine must isolate the tokio async runtime on dedicated threads, completely separate from the rayon compute pool and the main game thread.

## Solution

### Dedicated Tokio Runtime

At engine startup, create a multi-threaded tokio runtime on its own threads. The runtime is configured with a small thread count (2-4 threads, configurable) because I/O-bound work does not benefit from many cores -- it benefits from not blocking.

```rust
use tokio::runtime::Runtime;
use std::sync::Arc;

pub struct AsyncRuntime {
    runtime: Arc<Runtime>,
    shutdown_signal: tokio::sync::watch::Sender<bool>,
}

impl AsyncRuntime {
    pub fn new(thread_count: usize) -> Self {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(thread_count)
            .thread_name("nebula-async")
            .enable_all()
            .build()
            .expect("failed to create tokio runtime");

        Self {
            runtime: Arc::new(runtime),
            shutdown_signal: shutdown_tx,
        }
    }

    /// Spawn an async task on the tokio runtime.
    /// Returns a JoinHandle for the task.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }

    /// Get a handle that can be cloned and sent to other threads.
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Signal all tasks to shut down and wait for completion.
    pub fn shutdown(self) {
        log::info!("Shutting down async runtime...");
        let _ = self.shutdown_signal.send(true);

        // Drop the Arc<Runtime>, which triggers shutdown.
        // If other Arcs exist, this is a no-op until all are dropped.
        // Use shutdown_timeout for a bounded wait.
        match Arc::try_unwrap(self.runtime) {
            Ok(runtime) => {
                runtime.shutdown_timeout(std::time::Duration::from_secs(5));
                log::info!("Async runtime shut down cleanly");
            }
            Err(_arc) => {
                log::warn!(
                    "Async runtime has outstanding references, \
                     shutdown will complete when all references are dropped"
                );
            }
        }
    }
}
```

### Thread Isolation Guarantee

The tokio runtime's threads are completely separate from:

- The **main game thread** (thread 0, runs ECS, game loop, input)
- The **rayon thread pool** (stories 01, 05 -- runs meshing, terrain gen, physics)

This is enforced by construction: tokio creates its own OS threads via `tokio::runtime::Builder`, and rayon creates its own via `rayon::ThreadPoolBuilder`. They share no thread pool. The main thread is the thread that calls `main()` and is not part of either pool.

Typical thread allocation for an 8-core machine:

| Thread | Role |
|--------|------|
| Thread 0 | Main game thread (ECS, game loop, input) |
| Threads 1-2 | Tokio async I/O (networking, file I/O, asset loading) |
| Threads 3-7 | Rayon compute pool (meshing, terrain, physics) |
| Thread 8 (if HT) | Spare / OS |

### Communication with the Game Thread

The game thread never calls `.await` and never blocks on async work. Communication is strictly through crossbeam channels (story 02):

```rust
// On the tokio side: after receiving a network message
async fn network_recv_loop(
    stream: tokio::net::TcpStream,
    sender: crossbeam::channel::Sender<NetworkMessage>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 4096];

    loop {
        tokio::select! {
            result = stream.readable() => {
                match result {
                    Ok(()) => {
                        // Read and deserialize message...
                        let msg = NetworkMessage::Heartbeat { timestamp_ms: 0 };
                        if sender.try_send(msg).is_err() {
                            log::warn!("Network inbound channel full, dropping message");
                        }
                    }
                    Err(e) => {
                        log::error!("Network read error: {}", e);
                        break;
                    }
                }
            }
            _ = shutdown.changed() => {
                log::info!("Network receive loop shutting down");
                break;
            }
        }
    }
}
```

The main thread collects network messages each frame via `drain_channel(&hub.network_inbound.receiver)` (story 02). The main thread sends outbound messages by pushing them into the `network_outbound` channel, which the tokio network task drains and transmits.

### Async Asset Loading

Asset loading (textures, models, sounds) uses tokio for non-blocking file I/O:

```rust
async fn load_asset(
    path: std::path::PathBuf,
    sender: crossbeam::channel::Sender<AssetLoadResult>,
) {
    match tokio::fs::read(&path).await {
        Ok(data) => {
            let _ = sender.try_send(AssetLoadResult::Loaded {
                path,
                data,
            });
        }
        Err(e) => {
            let _ = sender.try_send(AssetLoadResult::Failed {
                path,
                error: e.to_string(),
            });
        }
    }
}
```

### Graceful Shutdown

On engine exit, the shutdown sequence is:

1. Send `true` through the `shutdown_signal` watch channel.
2. All tokio tasks observe the signal and break out of their loops.
3. Call `AsyncRuntime::shutdown()`, which calls `runtime.shutdown_timeout(5s)`.
4. If tasks do not complete within 5 seconds, they are forcibly dropped.
5. Rayon pool shutdown follows (rayon tasks check cancellation tokens, story 06).

This ordering ensures network connections send a clean disconnect message before the runtime terminates.

### Configuration

```toml
[threading.async]
worker_threads = 2       # Number of tokio worker threads
shutdown_timeout_secs = 5 # Maximum time to wait for async tasks during shutdown
```

On machines with fewer than 4 cores, reduce to 1 tokio thread.

## Outcome

An `AsyncRuntime` struct in the `nebula-threading` crate that wraps a multi-threaded tokio 1.49 runtime running on dedicated threads, fully isolated from the rayon compute pool and the main game thread. The runtime handles networking, file I/O, and asset loading. Communication with the game thread is exclusively through crossbeam channels. Graceful shutdown drains all async tasks within a configurable timeout.

## Demo Integration

**Demo crate:** `nebula-demo`

The tokio async runtime for networking runs on dedicated threads, isolated from the game loop. A network latency spike does not cause a frame drop.

## Crates & Dependencies

- **`tokio`** = `"1.49"` with features `["full"]` -- multi-threaded async runtime for I/O-bound tasks
- **`crossbeam`** = `"0.8"` -- channels for game thread communication (shared with story 02)
- **`log`** = `"0.4"` -- logging runtime lifecycle events
- Rust edition **2024**

## Unit Tests

- **`test_tokio_runtime_starts_on_separate_threads`** -- Create an `AsyncRuntime` with 2 threads. Spawn a task that records `std::thread::current().name()`. Assert the thread name starts with `"nebula-async"`. Assert the thread ID is different from the test's main thread ID.

- **`test_game_thread_does_not_block_on_async`** -- Create an `AsyncRuntime`. Spawn an async task that sleeps for 500ms (`tokio::time::sleep`). Immediately check that the main thread can proceed by measuring the elapsed time: assert it is less than 10ms. The async task runs independently in the background.

- **`test_channel_communication_async_to_sync`** -- Create a crossbeam channel and an `AsyncRuntime`. Spawn a tokio task that sends 5 messages through the crossbeam sender. On the main thread, drain the receiver (with a brief retry loop). Assert all 5 messages are received with correct values.

- **`test_channel_communication_sync_to_async`** -- Create a crossbeam channel and an `AsyncRuntime`. On the main thread, send 3 messages through the sender. Spawn a tokio task that receives from the crossbeam receiver. Collect results and assert all 3 messages are received correctly.

- **`test_runtime_shuts_down_cleanly`** -- Create an `AsyncRuntime`. Spawn a task that loops checking the shutdown signal. Call `shutdown()`. Assert that it returns within 6 seconds (5s timeout plus 1s margin). Assert no panic occurs.

- **`test_network_task_runs_on_tokio`** -- Create an `AsyncRuntime`. Spawn a task that binds a `tokio::net::TcpListener` on a random port. Assert the bind succeeds (proving tokio's network driver is active). Drop the listener and shut down.

- **`test_multiple_concurrent_async_tasks`** -- Spawn 100 async tasks that each increment an `Arc<AtomicU64>`. Await all tasks. Assert the counter equals 100, confirming all tasks ran to completion on the tokio runtime without interference.

- **`test_runtime_thread_count_matches_config`** -- Create an `AsyncRuntime` with `thread_count = 3`. Spawn 10 tasks that each record their thread name. Collect the unique thread names. Assert there are at most 3 unique `"nebula-async"` names (tokio may use fewer threads than configured if load is light, but never more).
