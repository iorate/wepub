# wepub-core

[![Crates.io](https://img.shields.io/crates/v/wepub-core.svg)](https://crates.io/crates/wepub-core)
[![Docs.rs](https://img.shields.io/docsrs/wepub-core)](https://docs.rs/wepub-core)
[![License](https://img.shields.io/crates/l/wepub-core.svg)](#license)

Asynchronous client library for publishing browser extensions to web stores.

`wepub-core` is the engine behind the [`wepub`](https://crates.io/crates/wepub) command-line tool. It exposes one `Client` type per supported store (Chrome Web Store, Firefox Add-ons, and Edge Add-ons) and a single `publish` verb that handles upload, validation polling and version submission end-to-end.

## Example

```rust
use wepub_core::firefox::{Channel, Client, Credentials, PublishOptions};

let client = Client::new(
    "myaddon@example.com".into(),
    Credentials {
        api_key: "user:12345:6789".into(),
        api_secret: "jwt-secret".into(),
    },
)?;
let zip = std::fs::read("./addon.zip")?;
client.publish(zip, Channel::Listed, PublishOptions::new()).await?;
```

See the full API on [docs.rs](https://docs.rs/wepub-core).

## License

Licensed under either of MIT or Apache-2.0, at your option.
