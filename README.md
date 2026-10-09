# Sameframe

A lightweight, anonymous watch-together app built with
[Topcoat](https://github.com/tokio-rs/topcoat), Rust, and Tokio. Catppuccin Mocha,
dark-only, with a responsive interface and no client framework.

## Watching together

- Create a temporary room and share a guest invite link.
- Keepers and Co-keepers load videos and control shared playback through YouTube's
  native play/pause/seek/speed controls.
- Volume and fullscreen use the native player controls and stay local.
- Members get shared random names and avatars, and rooms have random names.
- Guests follow the room's authoritative playback timeline. Personal playback
  changes can pause following; Rejoin playback returns to the shared timeline.
- Full snapshots repair missed updates and reconnects; buffering is local.
- Chat includes video suggestions and join/leave/playback activity.
- Keepers can grant or revoke Co-keeper permissions.
- No accounts, database, or playlist.

Host access is a random capability, distinct from the room link. The initial
host URL carries it in a fragment, which the browser removes immediately and
stores in session storage. Keep the host tab/session: the shared guest link
cannot restore host access. Room state is held in memory and disappears on a
server restart. Empty rooms expire after 30 minutes, including rooms that were
created but never joined.

## Private site access

Every page, API and WebSocket requires the shared site key, independent of room
host/moderator permissions. On first startup Sameframe securely generates 256
random bits and atomically saves 64 lowercase hex characters in
`./access-token` (file mode 0600; newly created parent directories 0700).
The startup log names **only the file**, never the key. Choose another path with
`--access-token-file PATH`. An existing malformed, unreadable, symlinked or
insecurely permissioned file stops startup rather than silently rotating access.

Read the key privately from that file and give it to your people separately from
clean room invite links. They can paste it into the lock screen or use
`https://your-site/?access_token=YOUR_KEY`. The lock screen removes that query
before unlocking; it never returns private content just because a URL key looks
valid. Browser localStorage remembers the key; a one-year HttpOnly, SameSite=Strict
cookie admits later requests. The cookie contains a domain-separated digest,
not the shared key. A query key is processed even when a cookie already exists.

Use **HTTPS outside localhost/127.0.0.1/[::1]**. Public-host cookies are Secure,
including when cloudflared connects to the backend over HTTP. Preserve the public
`Host`; arbitrary forwarded headers are not trusted. URLs can still reach browser
history, proxy/access logs or copied messages before JavaScript strips them:
configure infrastructure not to record query strings and prefer pasting the key.
Do not commit or publicly expose the token file or localStorage data.

To revoke access, stop the server, atomically replace the file with a new securely
generated 64-character lowercase hex key at mode 0600, then restart. All old
cookies and remembered keys cease working; distribute the new key privately.
Keep the file across ordinary restarts. This is one shared capability, not
individual accounts, and does not turn guests into room hosts.

### CLI configuration and systemd

```sh
sameframe                                      # Private; uses ./access-token
sameframe --access-token-file /var/lib/sameframe/access-token
sameframe --public                             # Disable site admission explicitly
sameframe --help
```

`--public` neither creates nor reads a token file. It leaves room Keeper/Co-keeper
permissions and same-origin checks intact, and conflicts with `--access-token-file`.
Configuration is via CLI arguments, not `SAMEFRAME_ACCESS_TOKEN_FILE`.

For systemd, use `StateDirectory=sameframe` and
`WorkingDirectory=/var/lib/sameframe` (with a service user's writable home/state
directory there). The default filename then resolves to
`/var/lib/sameframe/access-token`; no hidden subdirectory is created. Alternatively,
pass that absolute path in `ExecStart`. Preserve the state directory on restart.
To keep an existing development key from the previous layout, move
`.sameframe/access-token` to `./access-token` before restarting; there is no implicit
migration or old-path fallback.

## Development

```sh
nix develop
# Or, with direnv installed: direnv allow
topcoat dev
```

Open <http://127.0.0.1:3000> and unlock with the generated key. Topcoat rebuilds and reloads the page as you edit.
Override the bind address with `HOST` and `PORT`:

```sh
HOST=0.0.0.0 PORT=8080 topcoat dev
```

The shell provides Rust, the Topcoat CLI, Tailwind CSS v4, rust-analyzer, Node.js,
and jq. Pages use Topcoat UI and inline Tailwind classes, with system fonts.

## Architecture

- `src/rooms.rs`: bounded registry and one state-owning task per room. Commands
  are authorized, validated, revision-checked, and deduplicated. Latest-state
  watch channels coalesce updates instead of accumulating a playback backlog.
- `src/cli.rs`: explicit public mode and configurable access-token file path.
- `src/access.rs`: persistent shared site key, bounded unlock attempts, and a
  global cookie gate before routes or upgrades when private mode is enabled.
- `src/api.rs`: room creation and same-origin WebSockets. Join deadlines, global
  connection limits, per-peer creation limits, message limits, and timed socket
  writes keep slow connections out of the room actor.
- `src/pages.rs`, `styles.css`: server-rendered pages and the Mocha theme.
- `assets/room.js`, `assets/sync.js`: browser player bridge, clock estimation,
  reconnects, native-player interactions, and local drift correction. Native
  host actions become room commands; programmatic corrections, buffering, and
  autoplay rejection must not echo back into room commands.
- `docs/protocol.md`: wire format and recovery rules.

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

The flake supports Linux and macOS on x86_64 and aarch64. Nix flake builds include
only Git-tracked files; add new source files to the index before building. Avoid
`path:` builds of a working directory containing private token files.

## Operational limits

Sameframe is a single-process service: 256 rooms, 32 connections per
room, 512 total WebSockets, and bounded command queues and deduplication history.
Room creation is limited to eight requests per peer IP per minute. Behind a
reverse proxy, peers may share the proxy's IP; trusted proxy configuration and
edge abuse protection need to be set up before public deployment. These limits
bound resource use, but are not complete DDoS protection.

Serve pages and sockets from the same origin. A reverse proxy must preserve the
public `Host`, support WebSocket upgrades, and allow long-lived connections.
Nobody has to register: site admission is shared, while room host/moderator
capabilities remain separately authorised.
