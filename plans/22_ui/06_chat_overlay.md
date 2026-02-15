# Chat Overlay

## Problem

Multiplayer games require real-time text communication between players. The chat interface must be unobtrusive during gameplay (translucent, positioned out of the action area, messages fade over time) but must become the primary input target when the player wants to type. This creates a dual-mode challenge: in passive mode, the chat is a read-only overlay that does not capture input; in active mode, it captures all keyboard input for text entry, which means the engine's gameplay input systems must be suppressed without losing key state coherence. Messages must be displayed in chronological order, support scrolling through history, and integrate with the multiplayer networking system (Epic 18/19) to send and receive chat packets.

## Solution

Implement a `ChatOverlay` system in the `nebula_ui` crate that manages a message log and an input field using egui. The system operates in two modes controlled by a `ChatState` resource.

### ChatState

```rust
pub struct ChatState {
    pub mode: ChatMode,
    pub messages: VecDeque<ChatMessage>,
    pub max_visible: usize,        // default 10
    pub fade_timeout: f32,         // default 10.0 seconds
    pub scroll_offset: usize,      // 0 = bottom (most recent)
    pub input_buffer: String,
}

pub enum ChatMode {
    Passive,  // read-only, messages fade
    Active,   // text input is focused, keyboard captured
}

pub struct ChatMessage {
    pub sender: String,
    pub text: String,
    pub timestamp: f64,     // engine time when received
    pub color: egui::Color32,
}
```

### Passive Mode

When `ChatMode::Passive` is active:

1. The chat is drawn in `egui::Area::new("chat_overlay")` anchored to the bottom-left of the screen, offset by a margin (16px from left, 64px from bottom to clear the hotbar).

2. The last `max_visible` messages are displayed as `egui::Label` widgets with a semi-transparent background (`Color32::from_black_alpha(128)`). Each message is formatted as `"[sender]: text"`.

3. Messages fade based on age: `alpha = 1.0 - ((current_time - timestamp) / fade_timeout).clamp(0.0, 1.0)`. When alpha reaches 0, the message is still stored in the deque but not drawn. This means the chat area disappears entirely when all visible messages have faded.

4. When a new message arrives (from the network or from the local player), it is pushed to the back of the `VecDeque` and the fade timer starts fresh. If the deque exceeds a maximum history size (256 messages), the oldest message is popped from the front.

5. No keyboard input is captured. `EguiIntegration::wants_keyboard()` returns false.

### Active Mode

Pressing `Enter` or `T` (mapped in the input system) transitions to `ChatMode::Active`:

1. A `egui::TextEdit::singleline(&mut chat_state.input_buffer)` widget appears at the bottom of the chat area, below the message log. The text edit requests focus automatically via `response.request_focus()`.

2. The input context switches (via Epic 15, Story 05) so that all keyboard events are routed to egui and gameplay movement is suppressed. `EguiIntegration::wants_keyboard()` returns true.

3. All messages are displayed without fading (alpha forced to 1.0) so the player can read context while typing.

4. The message log becomes scrollable: mouse wheel or `PageUp`/`PageDown` adjusts `scroll_offset`, showing older messages above the visible window.

5. **Submit** -- Pressing `Enter` with a non-empty `input_buffer`:
   - Creates a `ChatMessage` with the local player's name, the input text, and the current timestamp.
   - Pushes the message to the local `messages` deque.
   - Emits a `SendChatEvent { text: String }` to the ECS event queue. The networking system (Epic 18) picks up this event and sends it to the server.
   - Clears `input_buffer`.
   - Transitions back to `ChatMode::Passive`.

6. **Cancel** -- Pressing `Escape` clears `input_buffer` and transitions to `Passive` without sending.

### Receiving Messages

An ECS system watches for `ReceivedChatEvent { sender: String, text: String }` events from the networking layer. When received, it constructs a `ChatMessage` and pushes it to `ChatState::messages`. The message appears immediately in the overlay.

### Scrolling

In active mode, `scroll_offset` controls which slice of `messages` is displayed. At offset 0, the most recent `max_visible` messages are shown. Increasing the offset scrolls up through history. The offset is clamped to `0..=messages.len().saturating_sub(max_visible)`. A small scroll indicator ("-- more --") appears at the top of the chat when there are older messages above the visible window.

## Outcome

A `chat_overlay.rs` module in `crates/nebula_ui/src/` exporting `chat_overlay_system`, `ChatState`, `ChatMessage`, `ChatMode`, `SendChatEvent`, and `ReceivedChatEvent`. The system draws the chat overlay during the UI construction phase every frame.

## Demo Integration

**Demo crate:** `nebula-demo`

A translucent chat area at the bottom of the screen shows recent messages. Pressing Enter opens the input field for typing.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: areas, labels, text edits, scroll areas |
| `serde` | `1.0` | Serialize/Deserialize `ChatMessage` for chat log persistence |
| `log` | `0.4` | Logging chat events, input mode transitions, message overflow |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_messages_display_in_order` | Push three messages with timestamps T=0, T=1, T=2 and render. | Messages appear top-to-bottom in chronological order (oldest at top, newest at bottom). |
| `test_messages_fade_after_timeout` | Push a message at T=0, advance engine time to T=11 (timeout=10). | The message's computed alpha is 0.0; it is not drawn in the egui output. |
| `test_new_message_resets_fade` | Push message A at T=0, push message B at T=5. Render at T=9. | Message A has alpha ~0.1 (nearly faded); message B has alpha ~0.6 (still visible). |
| `test_chat_input_captures_keyboard` | Transition to `ChatMode::Active`. | `EguiIntegration::wants_keyboard()` returns true; gameplay input is suppressed. |
| `test_submit_sends_message` | Type "hello" in the input buffer and press Enter. | A `SendChatEvent { text: "hello" }` is emitted. The message appears in the local log. `input_buffer` is empty. Mode returns to `Passive`. |
| `test_submit_empty_does_nothing` | Press Enter with an empty input buffer. | No `SendChatEvent` is emitted. Mode remains `Active`. |
| `test_escape_cancels_input` | Type "draft" in the input buffer and press Escape. | `input_buffer` is cleared. Mode returns to `Passive`. No event is emitted. |
| `test_scroll_shows_history` | Push 20 messages, set `max_visible = 10`, set `scroll_offset = 5`. | Messages 5 through 14 (0-indexed) are displayed instead of 10 through 19. |
| `test_scroll_offset_clamped` | Push 5 messages, set `max_visible = 10`, set `scroll_offset = 100`. | `scroll_offset` is clamped to 0 (all messages fit in one page). |
| `test_received_chat_event_adds_message` | Emit `ReceivedChatEvent { sender: "Player2", text: "hi" }`. | `ChatState::messages` contains a new entry with sender "Player2" and text "hi". |
| `test_max_history_evicts_oldest` | Push 257 messages (max history = 256). | `messages.len()` equals 256; the first message (oldest) has been evicted. |
| `test_passive_mode_no_keyboard_capture` | Set `ChatMode::Passive`. | `EguiIntegration::wants_keyboard()` returns false. |
