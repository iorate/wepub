# wepub-core

[![Crates.io](https://img.shields.io/crates/v/wepub-core.svg)](https://crates.io/crates/wepub-core)
[![Docs.rs](https://img.shields.io/docsrs/wepub-core)](https://docs.rs/wepub-core)
[![License](https://img.shields.io/crates/l/wepub-core.svg)](#license)

Asynchronous client library for publishing browser extensions to web stores.

`wepub-core` is the engine behind the [`wepub`](https://crates.io/crates/wepub) command-line tool. It exposes one `publish` entry point per supported store (Chrome Web Store, Firefox Add-ons, and Edge Add-ons), returning a builder that runs when finished with `call()`.

## Example

```rust
use wepub_core::firefox::{Channel, publish};

let package = std::fs::read("./addon.zip")?;
publish()
    .addon_id("myaddon@example.com")
    .api_key("user:12345:6789")
    .api_secret("jwt-secret")
    .package(package)
    .channel(Channel::Listed)
    .call()
    .await?;
```

See the full API on [docs.rs](https://docs.rs/wepub-core).

## License

Licensed under either of MIT or Apache-2.0, at your option.
