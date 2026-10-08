# Sameframe

A server-rendered [Topcoat](https://github.com/tokio-rs/topcoat) app. The starting
page is intentionally empty. Tailwind CSS and Topcoat UI are enabled, with the
stock neutral theme and Geist font.

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
and jq. Rust and Nix dependency pins match the reference `memexmd/www` setup.

## UI and styles

- `src/main.rs`: server entry point and empty home page.
- `styles.css`: Tailwind input and neutral theme tokens.
- `build.rs`: generates CSS with the shell's `tailwindcss` executable.
- `components.toml`: Topcoat UI registry state.

Add components when needed:

```sh
topcoat ui list
topcoat ui add button
```

## Checks and production build

```sh
topcoat fmt --rustfmt src build.rs
nix flake check
nix build
./result/bin/sameframe
```

The Crane build bundles stylesheets and any registered assets alongside the
server in `result/bin/assets`. Keep that directory with the binary when moving
the build. Geist font files use Fontsource's CDN, as in the reference project.
The package runs a live HTTP server; it does not export a static site.

The flake supports Linux and macOS on x86_64 and aarch64. It includes build,
Clippy, and Topcoat-aware formatting checks, but no workflows, secrets tooling,
or deployment configuration.

Nix flakes in a Git checkout only include tracked files. Until the initial files
are added to Git, use `nix develop "path:$PWD"`, `nix build "path:$PWD"`, or
`nix flake check "path:$PWD"`.
