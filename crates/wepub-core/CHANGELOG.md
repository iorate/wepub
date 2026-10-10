# wepub-core

## 1.1.0

### Minor Changes

- Bump MSRV to 1.89.0, required by uuid@1.27.0

## 1.0.2

### Patch Changes

- Fix `wepub edge` failing at the submit step with `[56] Failure when receiving data from the peer` when no notes are given. The request now sends `Content-Length: 0` instead of omitting the header, which the Edge Add-ons API rejected.
