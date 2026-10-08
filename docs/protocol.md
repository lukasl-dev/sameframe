# First slice: room protocol

Rooms are ephemeral; hosts and host-appointed moderators control playback. Guests
follow the room; a stalled guest
never pauses the room. All browser traffic is same-origin. No accounts or DB.

## HTTP

- `POST /api/rooms`, no body → 201 JSON `{ "room_id": "...", "host_token": "..." }`.
- `GET /room/{room_id}` → room page; unknown/expired room has a friendly unavailable page.
- `GET /api/rooms/{room_id}/ws` → WebSocket upgrade.

The creator navigates to `/room/{room_id}#host={host_token}`. The browser stores
that token in sessionStorage and immediately removes the fragment. The share
link has no fragment. Never put a host token in query strings, logs, snapshots,
or share links.

## WebSocket

First message, within 10 seconds:

```json
{"type":"join","host_token":null}
```

A matching host token grants host control; omission grants guest access. An
invalid token is rejected, not silently downgraded. Server sends:

```json
{"type":"welcome","member_id":1,"role":"guest","snapshot":{"incarnation":"random","room_name":"The Sleepy Observatory","revision":0,"media_revision":0,"video_id":null,"playing":false,"playback_rate":1.0,"position_secs":0.0,"anchor_ms":0,"members":1,"participants":[{"id":1,"name":"Sleepy Waffle","role":"guest","avatar":"0123456789abcdef"}],"events":[{"id":1,"at_ms":0,"member_id":1,"name":"Sleepy Waffle","avatar":"0123456789abcdef","kind":"joined","text":null,"video_id":null,"position_secs":null}]},"server_ms":0}
```

`member_id` identifies this connection in the public participant list. Each
participant has a server-assigned two-word funny name, unique within the room,
and a public random avatar seed, both fixed for that connection's lifetime.
`room_name` is a random cozy title assigned once, stable for the room's lifetime. Avatars are generated locally, with no third-party requests. A
reconnect creates a new anonymous membership; no persistent identity is implied.
Presence updates include the full participant list without changing playback
revision or anchor. Host tokens never appear in participant data.

`server_ms` and `anchor_ms` use the same process-monotonic millisecond clock.
Clients estimate the mapping to performance.now() using low-RTT ping samples:

```json
{"type":"ping","client_ms":123.4}
{"type":"pong","client_ms":123.4,"server_ms":150}
```

Clients ping periodically and request `{"type":"sync"}` after tab visibility
changes. The server also pushes complete state whenever room state changes:

```json
{"type":"snapshot","snapshot":{},"server_ms":150}
```

Host/moderator commands include an idempotency ID and expected playback revision:

```json
{"type":"command","id":"random-command-id","revision":0,"action":{"type":"set_video","video_id":"dQw4w9WgXcQ"}}
{"type":"command","id":"random-command-id","revision":1,"action":{"type":"play","position_secs":0.0}}
{"type":"command","id":"random-command-id","revision":2,"action":{"type":"pause","position_secs":12.5}}
{"type":"command","id":"random-command-id","revision":3,"action":{"type":"seek","position_secs":42.0}}
{"type":"command","id":"random-command-id","revision":4,"action":{"type":"set_rate","playback_rate":1.5}}
```

Set-video resets to paused at zero and sets `media_revision` to its accepted
playback revision. This identifies a reload even when the video ID is unchanged;
pausing or seeking to zero is not a reload. Loading also resets playback rate to
1x. Seek preserves playing/paused status. Set-rate re-anchors the current
position at the old rate before applying the new rate; it preserves playing or
paused status. Rates must be finite and between 0.25x and 4x, and the browser uses
only rates available from its player.
Accepted playback commands increment revision; membership, role changes, messages,
and proposals do not increment `revision` or `media_revision`, nor alter the
playback anchor/position/rate. Already-applied command IDs are acknowledged without
reapplying or appending another activity entry. Playback retries are scoped to the
authoring member; authorization is checked again before acknowledging a retry. Stale commands
are rejected and followed by current state. Never replay queued commands after
reconnect; fetch fresh state first. Only one command should be in flight per
client. While connected, rapid native interactions can coalesce into one latest
intent; discard it on reconnect, stale-state recovery, or a media change. Never
rebase an old video's intent onto a newly loaded video. Guests cannot issue
playback commands.

```json
{"type":"ack","id":"random-command-id","revision":4}
{"type":"error","code":"stale_revision","message":"Room state changed. Try again.","id":"random-command-id"}
```

Other error codes: forbidden, invalid_command, busy, unavailable, invalid_token.
Errors have `id: null` when not associated with a parsed command.

## Social commands and retained activity

Social commands are independent of playback: no expected revision, no playback
`ack`, and no automatic error-recovery snapshot. Every active member may chat or
propose a video. Proposals do **not** load or change playback. Only hosts may
change an active guest/moderator's role; hosts cannot be demoted or promoted.
The welcome role describes admission; thereafter derive the connection's current
role from `snapshot.participants` using `member_id`.

```json
{"type":"social","id":"chat-1","action":{"type":"message","text":"Hello room!"}}
{"type":"social","id":"proposal-1","action":{"type":"propose_video","video_id":"dQw4w9WgXcQ"}}
{"type":"social","id":"grant-1","action":{"type":"set_moderator","member_id":2,"enabled":true}}
{"type":"social","id":"revoke-1","action":{"type":"set_moderator","member_id":2,"enabled":false}}
{"type":"social_ack","id":"chat-1","event_id":3}
{"type":"error","code":"forbidden","message":"This member is not allowed to perform that action.","id":"grant-1"}
```

Messages are trimmed, nonempty, at most 500 Unicode scalar values and 2000 UTF-8
bytes after trimming. Video IDs use the same 11-character ASCII alphanumeric,
`_`, `-` validation as playback loads. IDs are nonempty ASCII strings of at most
64 bytes. Unknown fields are rejected, including nested action fields.
Social retry IDs have a separate 256-accepted-command window **per membership**;
playback retains its own bounded 256-command window. Retries consume the member's
rate token and return the original event ID without republishing/reapplying.
Reusing a retained ID with a different action fails; message comparison uses the
trimmed text. Once evicted, social IDs can create a new event. Never replay a
pending command after reconnecting into a new membership. An accepted role-command
retry is still acknowledged after its target departs, without applying it again.

Every snapshot retains the newest 100 `events`, oldest first, until room expiry:

```json
{"id":3,"at_ms":150,"member_id":2,"name":"Cosmic Waffle","avatar":"abcdef0123456789","kind":"message","text":"Hello room!","video_id":null,"position_secs":null}
```

These are the **exact nine fields**, with unused optional fields serialized as
`null`. `id` is a strictly increasing room-local u64, independent of playback
revision. `at_ms` shares the process-monotonic clock; it is not a wall timestamp.
Kinds are `joined`, `left`, `message`, `proposal`, `set_video`, `play`, `pause`,
`seek`, `set_rate`, and `role_changed`. Authors' names/avatar seeds are copied
into entries, so departure does not erase their identity. Role entries are
attributed to the host, with target name/new role in `text`; rate entries use
`text` such as `1.5x`. Proposals/loads include `video_id`; loads include position
zero, play/pause/seek include their accepted position. Membership and role events
have no playback position payload. Joining and leaving are logged once.

Clients should reconcile retained history by event ID on **every** snapshot,
even if playback revision is unchanged. Watch coalescing may omit intermediate
snapshots, but their events remain in the bounded tail. Gaps beyond that tail
are expected, not recoverable history. Render names, titles, and messages as
plain text, never HTML; event text is not HTML-sanitized by the server.

## Bounds and recovery

Latest-state watch channels coalesce updates; no playback backlog. Each socket
has a write timeout. Bounded actor mailbox, room/member caps, command rate limits,
4096-byte inbound frame/message limits, idle socket timeout, and empty-room expiry bound resource
use. Playback/social requests share the member's 20-token burst, 10-token/second
refill; the existing socket limiter and bounded response queue remain independent.
A reconnecting client joins anew and receives a complete snapshot.

The browser uses the authoritative anchor plus estimated server time to compute
the current position: while playing, add elapsed server time multiplied by
`playback_rate`; while paused, use the anchored position unchanged. Buffering, reconnect, player ready, and tab foreground
trigger reconciliation. Seek only for meaningful drift, with a cooldown;
correction events do not generate commands. Native host play, pause, and seek
actions send room commands. YouTube has no dedicated seek-intent event, so the
browser observes local position discontinuities with a programmatic-operation
guard; buffering lag must never be interpreted as a native seek or pause.

Guest native controls affect only their device. Personal pause/seek can suspend
following without changing the room. A visible Rejoin playback action returns
to the authoritative timeline. Volume and fullscreen always remain local.
Autoplay denial also needs a visible local Join playback action, not a global
room pause. Native host speed changes update the shared playback rate; local
programmatic rate adjustments must never echo as host input.
