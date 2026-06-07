# wepub

[![Crates.io](https://img.shields.io/crates/v/wepub.svg)](https://crates.io/crates/wepub)
[![CI](https://github.com/iorate/wepub/actions/workflows/ci.yml/badge.svg)](https://github.com/iorate/wepub/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/wepub.svg)](#license)

A CLI to publish browser extensions to web stores.

Chrome Web Store API v2, Firefox Add-ons API v5, and Edge Add-ons API v1.1 are supported.

It is used in [uBlacklist](https://github.com/iorate/ublacklist)'s release workflow to publish to all three stores.

## Examples

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
wepub edge ./my-addon.zip \
  --product-id "..." \
  --client-id  "..." \
  --api-key    "..."
```

## Install

GitHub Actions, via [setup-wepub](https://github.com/iorate/setup-wepub):

```yaml
uses: iorate/setup-wepub@v1
```

Shell script (macOS / Linux):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/iorate/wepub/releases/latest/download/wepub-installer.sh | sh
```

PowerShell (Windows):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/iorate/wepub/releases/latest/download/wepub-installer.ps1 | iex"
```

Homebrew:

```sh
brew install iorate/tap/wepub
```

npm:

```sh
npm install -g @iorate/wepub
```

Cargo (requires Rust 1.88+):

```sh
cargo install wepub
```

Prebuilt binaries for macOS, Linux, and Windows are also on the [release page](https://github.com/iorate/wepub/releases/latest).

## Usage

Only existing items can be updated; the initial submission of a new extension still has to go through each store's web UI.

### Chrome Web Store

Follow [Use the Chrome Web Store API](https://developer.chrome.com/docs/webstore/using-api) to obtain an OAuth client ID, client secret, and refresh token.

Alternatively, pass a pre-fetched OAuth access token via `--access-token` instead of a refresh token. This is suitable for automated workflows that authenticate with a [service account](https://developer.chrome.com/docs/webstore/service-accounts). The two authentication modes are mutually exclusive.

IDs and credentials are required and can be supplied via flags or environment variables:

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
| `--publish-type`      | Whether to publish immediately on approval (`default`) or stage for later publishing (`staged`).       |
| `--deploy-percentage` | Initial deploy percentage (0-100). Omit to use the Developer Dashboard default.                        |
| `--skip-review`       | Attempt to skip item review (`true` or `false`).                                                       |

### Firefox Add-ons

Obtain an API key and an API secret from the [API Credentials Management Page](https://addons.mozilla.org/developers/addon/api/key/).

IDs and credentials are required and can be supplied via flags or environment variables:

| Flag              | Environment variable           |
| ----------------- | ------------------------------ |
| `--addon-id`      | `WEPUB_FIREFOX_ADDON_ID`       |
| `--api-key`       | `WEPUB_FIREFOX_API_KEY`        |
| `--api-secret`    | `WEPUB_FIREFOX_API_SECRET`     |

Other flags:

| Flag                    | Description                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------- |
| `--channel`             | **Required.** Version channel (`listed` or `unlisted`). Determines visibility on the site.        |
| `--compatibility`       | Compatible applications, comma-separated (e.g. `firefox,android`).                                |
| `--approval-notes`      | Information for Mozilla reviewers. Mutually exclusive with `--approval-notes-file`.               |
| `--approval-notes-file` | Path to a file containing approval notes. Use `-` for stdin.                                      |
| `--release-notes`       | Release notes. Mutually exclusive with `--release-notes-file`.                                    |
| `--release-notes-file`  | Path to a file containing release notes. Use `-` for stdin.                                       |
| `--release-notes-lang`  | Locale code for the release notes (e.g. `en-US`, `ja`). Defaults to `en-US`.                      |
| `--source`              | Path to a source archive to attach to the version.                                                |

### Edge Add-ons

Obtain a client ID and an API key from the **Publish API** page of the [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login).

The product ID is the GUID shown on the **Extension overview** page in Partner Center.

IDs and credentials are required and can be supplied via flags or environment variables:

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

`wepub` reads a `.env` file from the current working directory at startup, so the `WEPUB_*` variables above can live there. Existing shell environment values take precedence over `.env` entries.

## License

Licensed under either of MIT or Apache-2.0, at your option.
