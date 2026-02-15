# Channel Architecture

## Problem

A multithreaded game engine has distinct threads with distinct responsibilities: the main game thread runs ECS schedules and orchestrates frames, worker threads perform CPU-bound meshing and terrain generation, the render thread prepares GPU uploads, and the network thread handles TCP connections. These threads must exchange data -- mesh results, generated chunks, GPU upload requests, and network messages -- without shared mutable state, without data races, and without blocking the main game thread.

Untyped or ad-hoc inter-thread communication leads to subtle bugs: a mesh result might be misinterpreted as a terrain result, an unbounded channel might accumulate thousands of pending messages and consume gigabytes of memory during a loading spike, or a blocking receive might stall the game thread waiting for a worker that is itself waiting for a different channel. The engine needs a disciplined, typed channel architecture with bounded capacities and non-blocking drain patterns.

## Solution

### Channel Definitions

All inter-thread channels are defined in a central `ChannelHub` struct created at engine startup and distributed (by cloning senders/receivers) to the subsystems that need them. Each channel is typed, bounded, and named.

```rust
use crossbeam::channel::{Sender, Receiver, bounded};

pub struct ChannelHub {
    pub mesh_result: ChannelPair<MeshResult>,
    pub chunk_gen_result: ChannelPair<ChunkGenResult>,
    pub gpu_upload_request: ChannelPair<GpuUploadRequest>,
    pub network_inbound: ChannelPair<NetworkMessage>,
    pub network_outbound: ChannelPair<NetworkMessage>,
}

pub struct ChannelPair<T> {
    pub sender: Sender<T>,
    pub receiver: Receiver<T>,
    pub name: &'static str,
    pub capacity: usize,
}

impl ChannelHub {
    pub fn new() -> Self {
        Self {
            mesh_result: ChannelPair::bounded("mesh_result", 256),
            chunk_gen_result: ChannelPair::bounded("chunk_gen_result", 128),
            gpu_upload_request: ChannelPair::bounded("gpu_upload_request", 64),
            network_inbound: ChannelPair::bounded("network_inbound", 512),
            network_outbound: ChannelPair::bounded("network_outbound", 512),
        }
    }
}

impl<T> ChannelPair<T> {
    pub fn bounded(name: &'static str, capacity: usize) -> Self {
        let (sender, receiver) = bounded(capacity);
        Self { sender, receiver, name, capacity }
    }
}
```

### Channel Descriptions

| Channel | Direction | Payload | Capacity | Purpose |
|---------|-----------|---------|----------|---------|
| `mesh_result` | Worker -> Main | `MeshResult` | 256 | Completed mesh vertex/index buffers ready for GPU upload |
| `chunk_gen_result` | Worker -> Main | `ChunkGenResult` | 128 | Generated voxel data for a chunk, ready for meshing or storage |
| `gpu_upload_request` | Main -> Render | `GpuUploadRequest` | 64 | Buffers that the render thread should upload to the GPU |
| `network_inbound` | Network -> Main | `NetworkMessage` | 512 | Deserialized messages received from remote peers |
| `network_outbound` | Main -> Network | `NetworkMessage` | 512 | Messages the main thread wants sent to remote peers |

### Message Types

Each channel carries a strongly-typed message. The compiler prevents accidentally sending a `MeshResult` through the `network_inbound` channel:

```rust
pub struct MeshResult {
    pub chunk_id: ChunkId,
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub lod_level: u8,
    pub frame_requested: u64,
}

pub struct ChunkGenResult {
    pub chunk_id: ChunkId,
    pub voxel_data: Box<ChunkData>,
    pub generation_time_us: u64,
}

pub struct GpuUploadRequest {
    pub buffer_id: BufferId,
    pub data: Vec<u8>,
    pub target: GpuBufferTarget,
}

pub enum NetworkMessage {
    PlayerPosition { player_id: u64, position: [f64; 3] },
    ChunkData { chunk_id: ChunkId, compressed: Vec<u8> },
    ChatMessage { sender_id: u64, text: String },
    Heartbeat { timestamp_ms: u64 },
    Disconnect { reason: String },
}
```

### Bounded Channel Semantics

All channels use `crossbeam::channel::bounded` with a configurable capacity. When a sender tries to push into a full channel:

- Worker threads use `try_send()` and log a warning on `TrySendError::Full`, dropping the message or retrying next tick. Worker threads must never block waiting for the main thread to drain.
- The main thread uses `try_send()` for GPU upload requests, ensuring it never blocks.
- Network threads use `send()` (blocking) for inbound messages with a large enough capacity (512) that blocking only occurs under extreme load, which itself is a signal to apply backpressure on the network.

```rust
match channel.sender.try_send(message) {
    Ok(()) => {},
    Err(crossbeam::channel::TrySendError::Full(msg)) => {
        log::warn!(
            "Channel '{}' is full (capacity {}), dropping message",
            channel.name,
            channel.capacity
        );
        // Optionally re-queue or apply backpressure
    },
    Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
        log::error!("Channel '{}' disconnected", channel.name);
    },
}
```

### Per-Frame Drain

The main game thread drains all inbound channels once per frame, at the start of the ECS update schedule. This collects all results produced by workers and network threads since the last frame:

```rust
pub fn drain_channel<T>(receiver: &Receiver<T>) -> Vec<T> {
    let mut results = Vec::new();
    while let Ok(msg) = receiver.try_recv() {
        results.push(msg);
    }
    results
}

pub fn drain_channels(hub: &ChannelHub) -> FrameMessages {
    FrameMessages {
        mesh_results: drain_channel(&hub.mesh_result.receiver),
        chunk_gen_results: drain_channel(&hub.chunk_gen_result.receiver),
        network_messages: drain_channel(&hub.network_inbound.receiver),
    }
}
```

The `FrameMessages` struct is inserted as an ECS resource, making all received data available to systems running in the current frame's schedule.

### Capacity Tuning

Channel capacities are configurable via the engine's configuration system (see `01_setup/07_configuration_system.md`). Defaults are tuned for typical gameplay, but during stress tests or on low-end hardware, operators can increase or decrease capacities:

```toml
[threading.channels]
mesh_result_capacity = 256
chunk_gen_result_capacity = 128
gpu_upload_request_capacity = 64
network_inbound_capacity = 512
network_outbound_capacity = 512
```

## Outcome

A `ChannelHub` struct in the `nebula-threading` crate that owns all inter-thread communication channels. Each channel is typed (compile-time safety against message mixing), bounded (memory-safe against runaway producers), and named (observable in debug overlays). A `drain_channel` utility collects pending messages non-blockingly each frame. Sending uses `try_send` on hot paths to guarantee the game thread never blocks on a full channel.

## Demo Integration

**Demo crate:** `nebula-demo`

Systems communicate via typed channels. The mesh system sends completed meshes to the render system via a channel — no shared mutable state between threads.

## Crates & Dependencies

- **`crossbeam`** = `"0.8"` -- bounded MPMC channels with `try_send`/`try_recv` support
- **`log`** = `"0.4"` -- warning on channel-full events and disconnection errors
- Rust edition **2024**

## Unit Tests

- **`test_message_roundtrip`** -- Create a `ChannelPair<u64>` with capacity 16. Send a value `42` through the sender. Receive from the receiver. Assert the received value equals `42`.

- **`test_bounded_channel_try_send_full`** -- Create a `ChannelPair<u8>` with capacity 2. Send two messages successfully. Attempt a third `try_send` and assert it returns `TrySendError::Full`. Verify the rejected message is returned in the error variant.

- **`test_typed_channels_prevent_mixing`** -- This is a compile-time test: attempt to send a `MeshResult` through a `Sender<NetworkMessage>`. Assert that this fails to compile. Document this as a compile-fail test using `trybuild` or as a commented-out assertion explaining the type safety guarantee.

- **`test_drain_collects_all_pending`** -- Create a channel with capacity 64. Send 10 messages from a separate thread. Wait briefly for delivery. Call `drain_channel` and assert exactly 10 messages are returned. Call `drain_channel` again and assert 0 messages are returned (the channel is now empty).

- **`test_drain_on_empty_channel`** -- Call `drain_channel` on a freshly created channel with no messages sent. Assert the returned `Vec` is empty and no panic occurs.

- **`test_multiple_producers_single_consumer`** -- Clone the sender 4 times. Spawn 4 threads, each sending 100 messages. Drain the channel and assert exactly 400 messages are received. Verify no messages are lost or duplicated by checking that all expected sequence numbers are present.

- **`test_channel_disconnection_detected`** -- Create a channel, drop the receiver. Attempt `try_send` on the sender and assert it returns `TrySendError::Disconnected`. Conversely, drop the sender, attempt `try_recv` on the receiver and assert it returns `TryRecvError::Disconnected`.

- **`test_channel_hub_construction`** -- Construct a `ChannelHub::new()` and verify all five channels are created with their expected names and capacities by inspecting the `name` and `capacity` fields.
