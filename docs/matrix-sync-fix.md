# Matrix Sync Token Bug Fix Plan

## Problem
The sync token advances even when no messages are extracted from allowed rooms. This causes messages to be missed when:
1. Sync returns events only for non-allowed rooms
2. Sync returns non-message events (typing, read receipts) for allowed rooms
3. Messages arrive during concurrent processing

## Root Cause
```rust
// Currently: Always save token after sync, even if no messages extracted
self.save_sync_token(&next_batch);
```

## Solution Options

### Option 1: Only advance token when messages extracted (WRONG)
This would cause infinite re-processing of non-message events.

### Option 2: Track "seen" event IDs (Complex)
Store event IDs of processed messages, skip duplicates. Memory grows over time.

### Option 3: Per-room sync tokens (Best)
Matrix supports per-room `prev_batch` tokens. Track last seen event per room.

### Option 4: Check if allowed rooms had ANY events (Simple fix)
Only advance token if allowed rooms were present in sync response, regardless of whether they had messages.

## Recommended Fix: Option 4

```rust
// Track whether any allowed rooms appeared in this sync
let mut allowed_rooms_in_sync = false;

if let Some(rooms) = sync_response.rooms {
    if let Some(joined_rooms) = rooms.join {
        for (room_id, room_data) in joined_rooms {
            let in_allowed = self.allowed_chats.contains(&room_id) || dm_rooms.contains(&room_id);
            if in_allowed {
                allowed_rooms_in_sync = true;
                // ... process messages as before
            }
        }
    }
}

// Only advance token if:
// 1. We extracted messages, OR
// 2. Allowed rooms were in sync (even with no messages = they're caught up)
// 3. No allowed rooms configured (process everything)
if !messages.is_empty() || allowed_rooms_in_sync || (self.allowed_chats.is_empty() && dm_rooms.is_empty()) {
    self.save_sync_token(&next_batch);
}
```

## Why This Works
- If allowed rooms appear in sync with no messages → token advances (caught up)
- If sync only has events for non-allowed rooms → token does NOT advance
- Next sync will include the same batch + any new events
- Eventually the allowed room will appear and token advances

## Edge Case: Stuck Token
If we never get events for allowed rooms, token never advances. This is fine — we'll get them eventually when someone sends a message.

## Implementation
File: `crates/rustyclaw-core/src/messengers/matrix_cli.rs`
Lines: ~455-460 (save_sync_token section)
