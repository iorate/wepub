# wepub

A CLI to publish browser extensions to web stores.

> **Status**: under development. Firefox (AMO) is implemented; Chrome Web Store and Edge Add-ons are planned.

## Install

`wepub` is not yet published to crates.io. Install from this repository:

```sh
cargo install --git https://github.com/iorate/wepub
```

Requires Rust 1.85+ (edition 2024).

## Usage

### Firefox (AMO)

Get a JWT credential pair from <https://addons.mozilla.org/developers/addon/api/key/>, then:

```sh
wepub firefox publish ./my-addon.zip \
  --addon-id   "myaddon@example.com" \
  --api-key    "user:1234567:89" \
  --api-secret "abcdef..." \
  --channel    listed
```

Credentials can also be supplied via environment variables:

| Flag             | Environment variable           |
| ---------------- | ------------------------------ |
| `--addon-id`     | `WEPUB_FIREFOX_ADDON_ID`       |
| `--api-key`      | `WEPUB_FIREFOX_API_KEY`        |
| `--api-secret`   | `WEPUB_FIREFOX_API_SECRET`     |
| `--amo-base-url` | `WEPUB_FIREFOX_AMO_BASE_URL`   |

Run `wepub firefox publish --help` for the full list of flags (compatibility, release notes, approval notes, source archive, etc.).

### Logging

- Default: `INFO` level (upload / validation progress visible)
- `-v` / `--verbose`: `DEBUG`
- `-q` / `--quiet`: `WARN+` only
- `RUST_LOG`: takes precedence (e.g. `RUST_LOG=trace`)

## Development

This is a Cargo workspace with two crates:

- `crates/wepub-core` — async library that talks to store APIs (built on `reqwest` + `tokio`)
- `crates/wepub` — CLI binary (`#[tokio::main]`, `clap`)

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Pre-commit hooks (`prek`) run `cargo fmt --check` and `cargo clippy` on Rust file changes.

## License

Licensed under either of MIT or Apache-2.0, at your option.
