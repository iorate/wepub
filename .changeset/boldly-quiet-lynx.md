---
wepub-core: minor
wepub: patch
---

Update jsonwebtoken to v12. An empty Firefox Add-ons API secret is now reported as an error before any request is sent, and `WepubError` gains the `FirefoxJwt` variant for this case.
