# Sameframe

A lightweight, anonymous watch-together app built with
[Topcoat](https://github.com/tokio-rs/topcoat), Rust, and Tokio. Catppuccin Mocha,
dark-only, with a responsive interface and no client framework.

## First slice

- Create a temporary room and share a guest invite link.
- The creator loads videos and uses YouTube's native play/pause/seek/speed controls.
- Volume and fullscreen use the native player controls and stay local.
- Members have locally generated random avatars and anonymous host/guest labels.
- Guests follow the room's authoritative playback timeline. Personal playback
  changes can pause following; Rejoin playback returns to the shared timeline.
- Full snapshots repair missed updates and reconnects; buffering is local.
- No accounts, database, chat, or playlist yet.

Host access is a random capability, distinct from the room link. The initial
host URL carries it in a fragment, which the browser removes immediately and
stores in session storage. Keep the host tab/session: the shared guest link
cannot restore host access. Room state is held in memory and disappears on a
server restart. Empty rooms expire after 30 minutes, including rooms that were
created but never joined.

## Development

```sh
nix develop
# Or, with direnv installed: direnv allow
topcoat dev
```

Open <http://127.0.0.1:3000>. Topcoat rebuilds and reloads the page as you edit.
Override the bind address with `HOST` and `PORT`:

```sh
HOST=0.0.0.0 PORT=8080 topcoat dev
```

The shell provides Rust, the Topcoat CLI, Tailwind CSS v4, rust-analyzer, Node.js,
and jq. The Rust toolchain and Nix input pins match the reference `memexmd/www`
setup. Topcoat UI and Tailwind are enabled; system fonts avoid another download.

## Architecture

- `src/rooms.rs`: bounded registry and one state-owning task per room. Commands
  are authorized, validated, revision-checked, and deduplicated. Latest-state
  watch channels coalesce updates instead of accumulating a playback backlog.
- `src/api.rs`: room creation and same-origin WebSockets. Join deadlines, global
  connection limits, per-peer creation limits, message limits, and timed socket
  writes keep slow connections out of the room actor.
- `src/pages.rs`, `styles.css`: server-rendered pages and the Mocha theme.
- `assets/room.js`, `assets/sync.js`: browser player bridge, clock estimation,
  reconnects, native-player interactions, and local drift correction. Native
  host actions become room commands; programmatic corrections, buffering, and
  autoplay rejection must not echo back into room commands.
- `docs/protocol.md`: wire format and recovery rules.

Topcoat's Rust-to-JavaScript runtime was considered. Its supported expression
vocabulary does not directly cover YouTube or WebSocket APIs; those still need
JavaScript interop. This slice keeps one small browser controller instead of
adding a second reactive state system around it. The runtime remains an option
for future UI where it actually simplifies the code.

The browser uses YouTube's official iframe API, loaded only after a video is
selected. Each viewer streams directly from YouTube; Sameframe does not proxy
media. Embedding restrictions, ads, network stalls, browser autoplay rules, and
background-tab throttling prevent a promise of frame-perfect synchronization.
A visible local Join playback action handles autoplay rejection.

## Tests and builds

```sh
cargo test
cargo clippy --all-targets -- -D warnings
node --test tests/client*.mjs
topcoat fmt --rustfmt src build.rs
nix flake check
nix build
./result/bin/sameframe
```

Rust tests cover the state machine and real multi-client WebSocket upgrades,
permissions, duplicates, stale revisions, reconnect snapshots, and malformed
messages. Browser helper tests cover clocks, snapshot ordering, URL parsing,
and correction policy without requiring YouTube or external services.
See `docs/testing.md` for the real-player and mobile smoke-test checklist.

The Crane build bundles CSS and browser modules alongside the server in
`result/bin/assets`. Keep that directory with the binary when moving the build.
This is a live HTTP server, not a static export.

The flake supports Linux and macOS on x86_64 and aarch64. No workflows, secrets
tooling, or deployment configuration are included. Until new files are added
to Git, use `nix develop "path:$PWD"`, `nix build "path:$PWD"`, or
`nix flake check "path:$PWD"` so Nix can see untracked source files.

## Operational limits

This first version is a single-process service: 256 rooms, 32 connections per
room, 512 total WebSockets, and bounded command queues and deduplication history.
Room creation is limited to eight requests per peer IP per minute. Behind a
reverse proxy, peers may share the proxy's IP; trusted proxy configuration and
edge abuse protection need to be set up before public deployment. These limits
bound resource use, but are not complete DDoS protection.

Serve pages and sockets from the same origin. A reverse proxy must preserve the
public `Host`, support WebSocket upgrades, and allow long-lived connections.
Host-only controls are deliberate for this slice; shared controls and host
transfer are future work. Nobody has to register or log in.
