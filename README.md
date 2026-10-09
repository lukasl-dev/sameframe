# sameframe

Watch YouTube together. Built with [Topcoat](https://github.com/tokio-rs/topcoat).

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
