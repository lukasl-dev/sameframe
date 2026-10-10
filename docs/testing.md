# Playback checks

Run `nix flake check` for Rust state/transport tests, client boundary tests,
Clippy, and formatting. These are deterministic and do not depend on YouTube.

The client harness models delayed iframe getters and setter callbacks. A passing
harness is not a substitute for testing YouTube's real iframe API.

## Site access coverage

Backend tests use `Access::for_test` and a synthetic 64-character lowercase hex
key, never production token files. HTTP coverage checks lock pages on every
path (including unmatched routes), HEAD, encoded/repeated query keys with and
without a cookie, no-referrer/no-store/Vary Cookie headers, strict bounded unlock
JSON, origin checks, one-year cookie flags, public Host security behind an HTTP
proxy, cookie probe, duplicate-cookie rejection, bounded/reusable attempt windows,
static-only public paths and traversal/method bypass attempts. Real socket tests
attempt raw unauthorised upgrades for existing/missing rooms with a URL key and
verify 401 JSON before upgrade; authenticated site guests still lack host powers.
Temporary-directory startup tests check 0600 files, newly created 0700 parents,
reload persistence, simultaneous starts, rotation and fail-closed handling of
malformed/unreadable/insecure files. Locked HTML and generic errors must not
contain even the known synthetic key.

For browser checks run a separate loopback server with
`--access-token-file` pointing to a temporary **0600** file containing a
synthetic key. Do not capture or print the actual development/service key.
Check wrong/manual key, URL-key stripping before requests, host-fragment
preservation, remembered-key restoration after clearing cookies, wrong-key
precedence over an existing cookie, blocked localStorage, blocked cookies (no
reload loop), generic unknown-room lock page, and 320px mobile layout. Inspect
network requests: only gate CSS/JS may load before admission; no room scripts or
YouTube embeds. Use two browser sessions admitted separately for room-role tests.

CLI tests check private-by-default behaviour, custom paths, explicit `--public`,
missing values, conflicting options and unknown arguments. Public-mode HTTP and
real socket tests check that visitors need no site cookie but same-origin checks
and room permissions remain enforced. Run `sameframe --public` in an empty temporary
working directory and verify that no token file is created; `--help` must also exit
without generating one. For systemd-style persistence, run from a temporary state
directory without CLI options and verify `access-token` is generated directly there
at mode 0600, then remains identical across a restart.

## Backend social/activity coverage

`cargo test` exercises the room actor and real WebSocket upgrades. Coverage includes:

- Host-only moderator grants/revocation, immutable host roles, inactive targets,
  all moderator playback actions, and authorization before dedupe after revocation.
- Shared stable two-word member names, uniqueness after vocabulary rollover,
  preserved avatar seeds, and a stable randomly assigned room title.
- Exact nine-field chat/proposal authorship, retained join/leave identities, and
  accepted playback events without retry duplication.
- Social/membership/role updates preserving playback revision, media generation,
  position, and anchor while playing; proposal submission never loads media.
- Coalesced watch updates retaining the ordered newest 100 events, persistence
  across an empty-room rejoin, and social retry history outliving the visible tail.
- Per-member social dedupe, conflicting IDs, separate playback/social retry stores,
  bounded eviction, invalid payloads, trimming/Unicode boundaries, rate refill,
  and escaped 500-character chat exceeding the former 2048-byte transport cap.
- `social_ack` versus existing error envelopes, no fabricated playback ack,
  nested unknown-field rejection, and the 4096-byte inbound cap.

The client harness covers same-revision role/feed updates, safe text rendering,
separate social acknowledgements, preserved drafts, suggestion approval,
Co-keeper playback, revoked authority, and chat scroll anchoring. Buffering must
never create a banner; routine notices occupy the fixed-height player caption.

## Reconnect regressions

Backend and socket tests check room-scoped signed identity tokens: stable name and
avatar across reconnects, fresh membership IDs, separate permissions/retry budgets,
invalid/cross-room rejection, concurrent tabs, and no token in public snapshots.
Browser tests check localStorage persistence, blocked-storage fallback, and one
fresh-identity retry without losing or granting Keeper access.

Playback harness tests delay corrective iframe writes across multiple samples and
deliver them one callback at a time. Same-incarnation welcomes must not fabricate
Play/Pause/Seek commands or detach Companions. Offline cached rate changes restore
the authoritative rate instead of sending commands. Visibility recovery waits for
fresh state, confirmed media loss reloads atomically, and undefined metadata is
rechecked rather than treated as loss. Native controls work after recovery; initial
Play-before-CUED, local detached watching, and autoplay-unlock Play still work.

## Browser smoke test

Use two independent browser sessions so the guest does not inherit the host's
session storage. Check desktop and a narrow mobile viewport.

1. Unlock the site independently in both sessions, then create a room. Its address must immediately become the clean guest invite
   URL; it must not contain a host token or query parameters.
2. Copy the link. The ghost icon briefly becomes a checkmark. Player position,
   document height, and visible notices must not change. With clipboard access
   blocked, the accessible feedback points to the address bar.
3. Load an embeddable YouTube video. It starts paused, with native controls and
   no custom seek field. The link input is above the player and has the same
   column width. Members remain below the video and visible on desktop;
   chat sits alongside it, then stacks below on mobile. Check 320px mobile,
   1366×768 desktop and a wide/short desktop. No horizontal scrolling.
4. Use the native Play button. Buffering must not cancel the Play action. Native
   Pause, timeline seeking, replay, and playback speed changes update the room.
5. Join after the host is already playing. The guest loads the correct current
   position without a false invalid-video error. Browser autoplay denial shows
   a local Join playback action rather than pausing the room.
6. Pause or seek as a guest. Only that device changes; it displays Watching
   locally and a Rejoin action. Rejoining catches up without a host command.
7. Stall one guest or take it offline. The host and other guests continue.
   Reconnect fetches the current snapshot; old commands are never replayed.
8. Select the same video again and then a different one. Old native events and
   queued scrub intentions cannot apply to the new media generation.
9. Try a non-embeddable/private video and block the iframe/API network request.
   Errors must be visible and local retry functional. No indefinitely empty
   player with apparently enabled controls.
10. Send a message and propose a video as a Companion (paste a YouTube link
    into either input). Both sessions see the same identities and history;
    only the Keeper/Co-keeper gets “Watch this together”. Suggestions alone
    never change playback. Grant and revoke Co-keeper from a member’s +/-
    button and verify permissions update without a reload.
11. Check join/leave, play/pause/seek and speed activity. Scroll up in chat:
    incoming messages must not pull you down; the new-message button returns
    to the bottom. Disconnect while sending: preserve the draft on failure.

Watch the console for application exceptions. The YouTube widget can emit
its own initialization warnings; distinguish those from application failures.

## Limits of native event attribution

YouTube does not attach a user/programmatic origin to state events and does not
provide a dedicated seek event. Sameframe observes local player discontinuities
and suppresses its own asynchronous operations. Tests should include buffering
before Playing, short Paused transitions around buffering, delayed setter
callbacks, fast consecutive scrubs, and a user interaction during startup.

Network stalls, ads, embedding restrictions, and background-tab throttling
prevent a guarantee of frame-perfect synchronization. Recovery should converge
when the player and connection become usable again, not slow down other viewers.
