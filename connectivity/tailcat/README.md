# Kratos Tailcat native adapter

This module is the production connectivity shim between Kratos's Rust HTTP/WebSocket peer and [Tailcat](https://github.com/tailscale/tailcat). It pins upstream commit `fd101889796a947ac514e9d86ec731af2965fad3` as Go pseudo-version `v0.6.1-0.20260913000754-fd101889796a`.

## Managed CLI contract

The parent process starts and owns one subprocess per role. Standard output contains exactly one readiness JSON line; diagnostics go to standard error and never include the secret Tailcat address.

```text
kratos-tailcat serve \
  --state /private/profile/server.key \
  --target 127.0.0.1:<rust-peer-port> \
  [--derp-map https://example/derpmap.json] [--region <id>]

=> {"address":"tc..."}
```

`serve` starts a persistent Tailcat server, listens only on Tailcat TCP application port `7332`, and proxies each connection only to the exact numeric IPv4 loopback target. It is not an exit node or unrestricted forwarder.

```text
kratos-tailcat connect \
  --config /private/profile/connect.json \
  --listen 127.0.0.1:0 \
  --state /private/profile/client.key \
  [--derp-map https://example/derpmap.json]

=> {"url":"http://127.0.0.1:<port>"}
```

`connect.json` has the exact schema `{"address":"tc..."}` and must be a regular mode-`0600` file. The secret is never accepted in argv. `connect` rejects unknown fields and trailing JSON, verifies connectivity before readiness, binds only numeric `127.0.0.1`, and sends every accepted stream to Tailcat application port `7332`. HTTP, streaming bodies, and WebSockets pass through as ordinary TCP bytes. SIGINT or SIGTERM closes listeners, active streams, and the Tailcat engine.

State is role-tagged so client/server identities cannot be interchanged. Secret files are atomically written with mode `0600` under a `0700` directory. Existing symlinks, non-regular files, incorrect permissions, malformed JSON, missing keys, and role mismatches fail closed rather than generating a replacement identity.

## Native binding contract

The root Go package is directly gomobile-bindable:

```go
client, err := tailcatnative.StartClient(address, stateDir, derpMap)
client.URL()
client.Close()

server, err := tailcatnative.StartServer(target, stateDir, derpMap)
server.Address()
server.Close()
```

The binding creates `client.key` or `server.key` in `stateDir`. It uses userspace networking only: no command execution, process spawning, privileged VPN, route changes, DNS changes, or arbitrary destination forwarding.


Release packages place `kratos-tailcat` adjacent to `zeron` (inside `Contents/MacOS` on macOS). The Windows ZIP is the supported portable download and contains both executables plus a checksum-bound preserve policy; the versioned standalone `zeron-...exe` release asset is updater payload for an existing ZIP installation, not a complete fresh installation. Windows uses the containing profile directory's private ACL because POSIX `0600` mode bits are not represented there.

## Build

From the repository root:

```sh
scripts/build-tailcat.sh native     # host CLI
scripts/build-tailcat.sh cross      # desktop/headless release matrix
scripts/build-tailcat.sh xcframework # macOS host; iOS device + simulator
scripts/build-tailcat.sh all
```

Artifacts are written under `target/tailcat/` by default. The iOS output is exactly `target/tailcat/KratosTailcat.xcframework`. Set `OUT_DIR` or `GO` to override those locations.
