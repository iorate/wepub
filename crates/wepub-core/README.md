# wepub-core

[![Crates.io](https://img.shields.io/crates/v/wepub-core.svg)](https://crates.io/crates/wepub-core)
[![Docs.rs](https://img.shields.io/docsrs/wepub-core)](https://docs.rs/wepub-core)
[![License](https://img.shields.io/crates/l/wepub-core.svg)](#license)

Asynchronous client library for publishing browser extensions to web stores.

`wepub-core` is the engine behind the [`wepub`](https://crates.io/crates/wepub) command-line tool. It exposes one `publish` entry point per supported store (Chrome Web Store, Firefox Add-ons, and Edge Add-ons), returning a builder that runs when `.await`ed.

## Example

```rust
use wepub_core::firefox::{Channel, Credentials};

let zip = std::fs::read("./addon.zip")?;
wepub_core::firefox::publish(
    "myaddon@example.com".into(),
    Credentials {
        api_key: "user:12345:6789".into(),
        api_secret: "jwt-secret".into(),
    },
    zip,
    Channel::Listed,
)
.await?;
```

See the full API on [docs.rs](https://docs.rs/wepub-core).

## License

Licensed under either of MIT or Apache-2.0, at your option.
