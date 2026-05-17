# wepub

[![Crates.io](https://img.shields.io/crates/v/wepub.svg)](https://crates.io/crates/wepub)
[![CI](https://github.com/iorate/wepub/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/wepub/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/wepub.svg)](#license)

A CLI to publish browser extensions to web stores.

Firefox (AMO), Chrome Web Store and Microsoft Edge Add-ons are supported.

## Install

```sh
cargo install wepub
```

Requires Rust 1.88+.

## Usage

### Firefox (AMO)

Get a JWT credential pair from <https://addons.mozilla.org/developers/addon/api/key/>, then:

```sh
wepub firefox ./my-addon.zip \
  --addon-id   "myaddon@example.com" \
  --api-key    "user:1234567:89" \
  --api-secret "abcdef..." \
  --channel    listed
```

Credentials can also be supplied via environment variables:

| Flag              | Environment variable           |
| ----------------- | ------------------------------ |
| `--addon-id`      | `WEPUB_FIREFOX_ADDON_ID`       |
| `--api-key`       | `WEPUB_FIREFOX_API_KEY`        |
| `--api-secret`    | `WEPUB_FIREFOX_API_SECRET`     |
| `--test-root-url` | `WEPUB_FIREFOX_TEST_ROOT_URL`  |

Run `wepub firefox --help` for the full list of flags (compatibility, release notes, approval notes, source archive, etc.).

**Note:** Only existing add-ons can be updated. The very first version of an add-on must still be uploaded through the AMO web UI.

### Chrome Web Store

Follow the [Chrome Web Store API setup guide](https://developer.chrome.com/docs/webstore/using-api) to obtain an OAuth client ID, client secret and refresh token, then:

```sh
wepub chrome ./my-extension.zip \
  --publisher-id  "12345678-90ab-cdef-1234-567890abcdef" \
  --item-id       "abcdefghijklmnopabcdefghijklmnop" \
  --client-id     "...apps.googleusercontent.com" \
  --client-secret "..." \
  --refresh-token "1//0..."
```

Alternatively, supply a pre-fetched OAuth access token (e.g. from `gcloud auth print-access-token` or a Workload Identity Federation flow) via `--access-token`. The two authentication modes are mutually exclusive.

Credentials and IDs can also be supplied via environment variables:

| Flag               | Environment variable           |
| ------------------ | ------------------------------ |
| `--publisher-id`   | `WEPUB_CHROME_PUBLISHER_ID`    |
| `--item-id`        | `WEPUB_CHROME_ITEM_ID`         |
| `--client-id`      | `WEPUB_CHROME_CLIENT_ID`       |
| `--client-secret`  | `WEPUB_CHROME_CLIENT_SECRET`   |
| `--refresh-token`  | `WEPUB_CHROME_REFRESH_TOKEN`   |
| `--access-token`   | `WEPUB_CHROME_ACCESS_TOKEN`    |
| `--test-root-url`  | `WEPUB_CHROME_TEST_ROOT_URL`   |
| `--test-token-url` | `WEPUB_CHROME_TEST_TOKEN_URL`  |

Run `wepub chrome --help` for the full list of flags (publish type, deploy percentage, skip review, etc.).

**Note:** Only existing items can be updated. New items must still be created through the Chrome Web Store Developer Dashboard.

### Microsoft Edge Add-ons

Enable the Update REST API at the [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login) (Microsoft Edge → Publish API → **Create API credentials**) to obtain a Client ID and an API key, then:

```sh
wepub edge ./my-extension.zip \
  --product-id "12345678-90ab-cdef-1234-567890abcdef" \
  --client-id  "..." \
  --api-key    "..."
```

The product ID is the GUID shown on the **Extension overview** page in Partner Center.

Credentials and IDs can also be supplied via environment variables:

| Flag              | Environment variable        |
| ----------------- | --------------------------- |
| `--product-id`    | `WEPUB_EDGE_PRODUCT_ID`     |
| `--client-id`     | `WEPUB_EDGE_CLIENT_ID`      |
| `--api-key`       | `WEPUB_EDGE_API_KEY`        |
| `--test-root-url` | `WEPUB_EDGE_TEST_ROOT_URL`  |

Run `wepub edge --help` for the full list of flags (notes for the certification team, etc.).

**Note:** Only existing products can be updated. The very first version of a product must still be created and published through Partner Center.

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
