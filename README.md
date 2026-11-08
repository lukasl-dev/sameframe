# sameframe

Watch videos together. Built with [Topcoat](https://github.com/tokio-rs/topcoat).

## Run

```sh
nix build
./result/bin/sameframe
```

Open <http://localhost:3000>. On first startup, the server creates `access-token`
in its working directory. Paste the token into the unlock screen or open a page
with `?access_token=TOKEN`. The browser remembers access.

```sh
sameframe --access-token-file /var/lib/sameframe/access-token
sameframe --public
```

Use HTTPS for public hosting. Rooms disappear when the server restarts.

## Videos

Paste a YouTube link or a direct `.mp4`, `.webm`, `.ogv`, `.ogg`, `.m4v`, or `.mov`
HTTP(S) URL. Direct videos load in each viewer's browser, not through the server;
the link must be reachable by everyone and its codecs supported by their browser.
The media host should support byte-range requests for seeking.
Use HTTPS media on an HTTPS site. Links requiring a website login, arbitrary
webpages, HLS/DASH playlists and DRM players are not supported.

Use the player's native controls. Keeper and Co-keeper play, pause, seek and speed
changes apply to the room on both providers. Companion changes stay local until
Rejoin playback. Direct links can also be suggested through chat. Signed URL
query parameters are shared with the room; do not paste personal credentials.

## Develop

```sh
nix develop
topcoat dev
```

## NixOS

```nix
imports = [ inputs.sameframe.nixosModules.default ];
services.sameframe.enable = true;
```

Defaults: `127.0.0.1:3000`, private access, state in `/var/lib/sameframe`.
Options: `package`, `host`, `port`, `public`, `dataDir`, `openFirewall`, `extraArgs`.

## Check

```sh
nix flake check
```

[Protocol](docs/protocol.md) · [Browser testing](docs/testing.md)
