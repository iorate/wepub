# Changelog

## [0.8.1](https://github.com/iorate/wepub/compare/v0.8.0...v0.8.1)

### 📚 Documentation


- State the supported store API versions in the README ([#77](https://github.com/iorate/wepub/pull/77)) - ([d07a451](https://github.com/iorate/wepub/commit/d07a451a42369b3e2975d6217dfeaebb60f77402))
- Mention uBlacklist's use in the README intro ([#75](https://github.com/iorate/wepub/pull/75)) - ([c3d211d](https://github.com/iorate/wepub/commit/c3d211d80edc43426f78df228f9e0a677ab26445))


## [0.8.0](https://github.com/iorate/wepub/compare/v0.7.1...v0.8.0)

### ⛰️ Features


- [**breaking**] Redesign progress and error output around tracing ([#72](https://github.com/iorate/wepub/pull/72)) - ([98ab89e](https://github.com/iorate/wepub/commit/98ab89e241acbac30156db5150b41769364274dc))

### 📚 Documentation


- Align the Edge with_poll_config doc comment with the other stores ([#74](https://github.com/iorate/wepub/pull/74)) - ([1a44d5e](https://github.com/iorate/wepub/commit/1a44d5ed9d5f6aec4053f7e404944506daff73d7))


## [0.7.1](https://github.com/iorate/wepub/compare/v0.7.0...v0.7.1)

### 🐛 Bug Fixes


- Group global options under their own help heading ([#68](https://github.com/iorate/wepub/pull/68)) - ([98ce56c](https://github.com/iorate/wepub/commit/98ce56ce874284fb31b1fe25c5a63400d8e60590))


## [0.7.0](https://github.com/iorate/wepub/compare/v0.6.1...v0.7.0)

### ⛰️ Features


- *(firefox)* [**breaking**] Rename the validation error's uuid field to upload_uuid ([#57](https://github.com/iorate/wepub/pull/57)) - ([602a652](https://github.com/iorate/wepub/commit/602a6524af84493c704ab35fbebb6011c164767b))
- Mark Progress and WepubError as non_exhaustive ([#67](https://github.com/iorate/wepub/pull/67)) - ([0ae9abe](https://github.com/iorate/wepub/commit/0ae9abe175421aef721aeafdb40f4cb94e4a5eb0))
- [**breaking**] Move publish progress from tracing to an on_progress callback ([#51](https://github.com/iorate/wepub/pull/51)) - ([08e1f18](https://github.com/iorate/wepub/commit/08e1f18b0a28abaea1689af89f22b9ac27236a6d))

### 📚 Documentation


- Add setup-wepub GitHub Action to install options ([#53](https://github.com/iorate/wepub/pull/53)) - ([d89177c](https://github.com/iorate/wepub/commit/d89177cf9a6adba4bd2c7a4e4d3793954274f4db))
- Fix publish example in README ([#63](https://github.com/iorate/wepub/pull/63)) - ([58edca3](https://github.com/iorate/wepub/commit/58edca33d4daa82ad67029ead21ffc6cd2a7b3ea))


## [0.6.1](https://github.com/iorate/wepub/compare/v0.6.0...v0.6.1)

### 📚 Documentation


- Trim redundant doc comments and align store API references ([#49](https://github.com/iorate/wepub/pull/49)) - ([22e4898](https://github.com/iorate/wepub/commit/22e48983f1fe524bce0e9d22a8fd3c1376960161))


## [0.6.0](https://github.com/iorate/wepub/compare/v0.5.1...v0.6.0)

### 🐛 Bug Fixes


- [**breaking**] Remove WepubError::Internal in favor of expect on impossible states ([#46](https://github.com/iorate/wepub/pull/46)) - ([5a8eca3](https://github.com/iorate/wepub/commit/5a8eca3a829f73868c9203f7c19590abd490e9b3))


## [0.5.1](https://github.com/iorate/wepub/compare/v0.5.0...v0.5.1)

### 📚 Documentation


- Overhaul README and correct Firefox compatibility doc comment ([#43](https://github.com/iorate/wepub/pull/43)) - ([2b89959](https://github.com/iorate/wepub/commit/2b89959de1d116c5f7520e15a09187b02a5f8fed))


## [0.5.0](https://github.com/iorate/wepub/compare/v0.4.3...v0.5.0)

### ⛰️ Features


- [**breaking**] Introduce per-store Credentials type and tidy Client APIs ([#39](https://github.com/iorate/wepub/pull/39)) - ([6b9f152](https://github.com/iorate/wepub/commit/6b9f1526da9d6beffe9afc7761149f65523b6426))

### 📚 Documentation


- Tidy store credential setup links ([#41](https://github.com/iorate/wepub/pull/41)) - ([cdc6fb8](https://github.com/iorate/wepub/commit/cdc6fb8edf7a82924deb0eb10cbd83e9b647b6da))


## [0.4.3](https://github.com/iorate/wepub/compare/v0.4.2...v0.4.3)

### 🐛 Bug Fixes


- Warn when .env exists but fails to load ([#37](https://github.com/iorate/wepub/pull/37)) - ([8f07274](https://github.com/iorate/wepub/commit/8f07274baff199321040a180699d2cb646ee5237))


## [0.4.2](https://github.com/iorate/wepub/compare/v0.4.1...v0.4.2)

### ⛰️ Features


- *(firefox)* Allow choosing release notes locale ([#25](https://github.com/iorate/wepub/pull/25)) - ([bcaea47](https://github.com/iorate/wepub/commit/bcaea4793fd37e5081efc0cd8c77d2a82402f11a))

### 📚 Documentation


- Refresh readme and tighten cli flag descriptions ([#29](https://github.com/iorate/wepub/pull/29)) - ([4248272](https://github.com/iorate/wepub/commit/424827209ab650040d8c44adc7f1900ee19dfdbf))
- Add wepub-core readme and tighten publish options docs ([#31](https://github.com/iorate/wepub/pull/31)) - ([69c25f6](https://github.com/iorate/wepub/commit/69c25f620f3933f009537032a13181580860b837))


## [0.4.1](https://github.com/iorate/wepub/compare/v0.4.0...v0.4.1)

### 🐛 Bug Fixes


- *(edge)* Submit publish notes as plain text ([#21](https://github.com/iorate/wepub/pull/21)) - ([534922d](https://github.com/iorate/wepub/commit/534922d69d03f406d0bcb8563bfa2c2f29565c8d))

### 📚 Documentation


- Remove redundant Errors sections from rustdoc ([#23](https://github.com/iorate/wepub/pull/23)) - ([496840a](https://github.com/iorate/wepub/commit/496840a76b78e89f96c2901a81c8407deced37d5))


## [0.4.0](https://github.com/iorate/wepub/compare/v0.3.0...v0.4.0)

### ⛰️ Features


- [**breaking**] Add Edge Add-ons support (with cross-cutting refactors) ([#18](https://github.com/iorate/wepub/pull/18)) - ([55ef595](https://github.com/iorate/wepub/commit/55ef595e989716dafafe270a528722a570d59381))


## [0.3.0](https://github.com/iorate/wepub/compare/v0.2.0...v0.3.0)

### ⛰️ Features


- [**breaking**] Rework test URL env vars and flags ([#10](https://github.com/iorate/wepub/pull/10)) - ([02997fa](https://github.com/iorate/wepub/commit/02997fafad574f87348d25cf89e1bda71dda1ebd))

### 📚 Documentation


- Replace blockquotes with note labels ([#14](https://github.com/iorate/wepub/pull/14)) - ([200b6e9](https://github.com/iorate/wepub/commit/200b6e932fe651bf904a503979c4f00dab2d6e13))
- Drop redundant --workspace from cargo commands ([#12](https://github.com/iorate/wepub/pull/12)) - ([db5e918](https://github.com/iorate/wepub/commit/db5e9185d2faae684d8e2c49d8f5c76c10bf3b85))

