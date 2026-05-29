# wepub

[![Crates.io](https://img.shields.io/crates/v/wepub.svg)](https://crates.io/crates/wepub)
[![CI](https://github.com/iorate/wepub/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/wepub/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/wepub.svg)](#license)

A CLI to publish browser extensions to web stores.

Chrome Web Store, Firefox Add-ons and Edge Add-ons are supported. Only existing items can be updated; the initial submission of a new extension still has to go through each store's web UI or developer dashboard.

## Install

```sh
cargo install wepub
```

Requires Rust 1.88+.

## Quick start

### Chrome Web Store

```sh
wepub chrome ./my-extension.zip \
  --publisher-id  "..." \
  --item-id       "..." \
  --client-id     "..." \
  --client-secret "..." \
  --refresh-token "..."
```

### Firefox Add-ons

```sh
wepub firefox ./my-addon.zip \
  --addon-id   "..." \
  --api-key    "..." \
  --api-secret "..." \
  --channel    listed
```

### Edge Add-ons

```sh
wepub edge ./my-extension.zip \
  --product-id "..." \
  --client-id  "..." \
  --api-key    "..."
```

## Usage

### Chrome Web Store

Follow [Use the Chrome Web Store API](https://developer.chrome.com/docs/webstore/using-api) to obtain an OAuth client ID, client secret and refresh token.

Alternatively, pass a pre-fetched OAuth access token via `--access-token` instead of a refresh token. This is suitable for automated workflows that authenticate with a [service account](https://developer.chrome.com/docs/webstore/service-accounts). The two authentication modes are mutually exclusive.

Credentials and IDs can also be supplied via environment variables:

| Flag               | Environment variable           |
| ------------------ | ------------------------------ |
| `--publisher-id`   | `WEPUB_CHROME_PUBLISHER_ID`    |
| `--item-id`        | `WEPUB_CHROME_ITEM_ID`         |
| `--client-id`      | `WEPUB_CHROME_CLIENT_ID`       |
| `--client-secret`  | `WEPUB_CHROME_CLIENT_SECRET`   |
| `--refresh-token`  | `WEPUB_CHROME_REFRESH_TOKEN`   |
| `--access-token`   | `WEPUB_CHROME_ACCESS_TOKEN`    |

Other flags:

| Flag                  | Description                                                                                            |
| --------------------- | ------------------------------------------------------------------------------------------------------ |
| `--publish-type`      | Whether to publish on approval (`default`) or stage for later publishing (`staged`).                   |
| `--deploy-percentage` | Initial deploy percentage (0-100). Omit to use the Developer Dashboard default.                        |
| `--skip-review`       | Attempt to skip item review (`true` or `false`).                                                       |

### Firefox Add-ons

Get a JWT credential pair from the [API Credentials Management Page](https://addons.mozilla.org/developers/addon/api/key/).

Credentials can also be supplied via environment variables:

| Flag              | Environment variable           |
| ----------------- | ------------------------------ |
| `--addon-id`      | `WEPUB_FIREFOX_ADDON_ID`       |
| `--api-key`       | `WEPUB_FIREFOX_API_KEY`        |
| `--api-secret`    | `WEPUB_FIREFOX_API_SECRET`     |

Other flags:

| Flag                    | Description                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------- |
| `--channel`             | **Required.** Version channel (`listed` or `unlisted`). Determines visibility on the site.        |
| `--compatibility`       | Compatible applications, comma-separated (`firefox`, `android`).                                  |
| `--approval-notes`      | Information for Mozilla reviewers. Mutually exclusive with `--approval-notes-file`.               |
| `--approval-notes-file` | Path to a file containing approval notes. Use `-` for stdin.                                      |
| `--release-notes`       | Release notes. Mutually exclusive with `--release-notes-file`.                                    |
| `--release-notes-file`  | Path to a file containing release notes. Use `-` for stdin.                                       |
| `--release-notes-lang`  | Locale code for the release notes (e.g. `en-US`, `ja`). Defaults to `en-US`.                      |
| `--source`              | Path to a source archive to attach to the version.                                                |

### Edge Add-ons

Enable the Update REST API at the [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login) (Microsoft Edge → Publish API → **Create API credentials**) to obtain a Client ID and an API key.

The product ID is the GUID shown on the **Extension overview** page in Partner Center.

Credentials and IDs can also be supplied via environment variables:

| Flag              | Environment variable        |
| ----------------- | --------------------------- |
| `--product-id`    | `WEPUB_EDGE_PRODUCT_ID`     |
| `--client-id`     | `WEPUB_EDGE_CLIENT_ID`      |
| `--api-key`       | `WEPUB_EDGE_API_KEY`        |

Other flags:

| Flag           | Description                                                                  |
| -------------- | ---------------------------------------------------------------------------- |
| `--notes`      | Notes for certification. Mutually exclusive with `--notes-file`.             |
| `--notes-file` | Path to a file containing notes for certification. Use `-` for stdin.        |

### `.env` file

`wepub` reads a `.env` file from the current working directory at startup. Any `KEY=VALUE` lines populate the process environment for subsequent flag resolution, so the `WEPUB_*` variables documented above can live in `.env` alongside your project. Existing shell environment values take precedence over `.env` entries.

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
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

Pre-commit hooks (`prek`) run `cargo fmt --check` and `cargo clippy` on Rust file changes.

## License

Licensed under either of MIT or Apache-2.0, at your option.
