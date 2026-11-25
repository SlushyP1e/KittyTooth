# Recorded API messages

Sample frames used by the unit tests. Pulsoid frames are copied from the official examples at
https://docs.pulsoid.net/heart-rate-and-sensor-data/read-data-via-websocket-v2 and
https://docs.pulsoid.net/heart-rate-and-sensor-data/read-heart-rate-via-websocket (v1), and
the error body from https://docs.pulsoid.net/reference/error-code-format.
`hyperate_hr_update.json` is the example from HypeRate's official
WEBSOCKET_API.md. HypeRate doesn't document its `phx_reply` frames, so the two reply fixtures
follow the standard Phoenix channel reply shape (`{"status": "ok" | "error", "response": {...}}`).
