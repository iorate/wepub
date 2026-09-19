---
wepub-core: patch
wepub: patch
---

Fix `wepub edge` failing at the submit step with `[56] Failure when receiving data from the peer` when no notes are given. The request now sends `Content-Length: 0` instead of omitting the header, which the Edge Add-ons API rejected.
