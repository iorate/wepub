# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`wepub` is a CLI to publish browser extensions to Chrome Web Store, Firefox Add-ons, and Edge Add-ons. Cargo workspace: `wepub-core` (async library, one `Client` per store with a single `publish` verb) and `wepub` (the `clap`-based binary).

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.

## Handling secrets

Prevent secret leakage. Authentication headers and the request/response bodies of credential-exchange endpoints (e.g. OAuth token refresh) carry secrets and must not be logged. Types holding secrets must not have implicit output derives like `#[derive(Debug)]` or `#[derive(Serialize)]`.
