# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

`wepub` is a CLI to publish browser extensions to Chrome Web Store, Firefox Add-ons, and Edge Add-ons. Cargo workspace: `wepub-core` (async library, one `Client` per store with a single `publish` verb) and `wepub` (the `clap`-based binary).

## Verifying Changes

After editing, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.

## Handling secrets

Prevent secret leakage.

Authentication headers and the request/response bodies of credential-exchange endpoints (e.g. OAuth token refresh) carry secrets and must not be logged.

Secret values must never reach `Debug` or `Serialize` output. A type that directly holds a secret string must redact it with a hand-written `Debug` impl that emits no field contents (e.g. `f.debug_struct("Credentials").finish_non_exhaustive()`) rather than `#[derive(Debug)]`. Never derive `Serialize` on a secret-holding type.
