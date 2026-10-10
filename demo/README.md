# Demo Assets

Public demo assets for the repository live in this directory.

## Scenarios

- `chat`: interactive CLI session against a live backend.
- `dashboard`: real-time TUI dashboard refresh cycle.
- `status`: health, skill listing, and session discovery.
- `send`: one-shot request through the custom HTTP adapter.

Each scenario is recorded from a `.tape` file and rendered into:

- `demo/orka-*.mp4`: browser-friendly video output.
- `demo/orka-*.webm`: site-friendly compressed alternative.
- `demo/orka-*.gif`: lightweight README preview asset.

Master recordings are staged under `demo/.build/` and can be removed with `./scripts/demo.sh clean`.

## Live Backend

The demo pipeline assumes a live Orka backend. Direct URLs default to
localhost (`http://localhost:18080` and `http://localhost:18081`).

`ORKA_DEMO_USE_SSH_TUNNEL` defaults to `1`. When tunnel mode is enabled,
`ORKA_DEMO_SSH_HOST` is required (no default). The recorder then talks to a
temporary local forward and does not need that host to resolve on the recorder
machine.

Disable the tunnel only if the backend is directly reachable. Set
`ORKA_DEMO_SERVER_URL` and `ORKA_DEMO_ADAPTER_URL` explicitly when you are not
targeting localhost. Set `ORKA_DEMO_API_KEY` if the target backend is
protected. The script also honors `ORKA_DEMO_ORKA_BIN` if you want to pin a
specific `orka` CLI binary.

| Variable | Required | Default | Description |
| -------- | -------- | ------- | ----------- |
| `ORKA_DEMO_USE_SSH_TUNNEL` | no | `1` | Open a dedicated SSH tunnel first |
| `ORKA_DEMO_SSH_HOST` | when `ORKA_DEMO_USE_SSH_TUNNEL=1` | none | SSH host used for the tunnel |
| `ORKA_DEMO_SERVER_URL` | no | `http://localhost:18080` | Direct demo server URL when the tunnel is disabled |
| `ORKA_DEMO_ADAPTER_URL` | no | `http://localhost:18081` | Direct demo adapter URL when the tunnel is disabled |
| `ORKA_DEMO_REMOTE_SERVER_PORT` | no | `18080` | Remote Orka API port forwarded by the tunnel |
| `ORKA_DEMO_REMOTE_ADAPTER_PORT` | no | `18081` | Remote adapter port forwarded by the tunnel |
| `ORKA_DEMO_API_KEY` | if the backend is protected | none | API key for the target backend |

```bash
# Tunnel to a reachable SSH host (required when tunnel mode is on)
export ORKA_DEMO_SSH_HOST=<host>
./scripts/demo.sh build

# Local backend, no tunnel
export ORKA_DEMO_USE_SSH_TUNNEL=0
./scripts/demo.sh build
```

## Commands

```bash
# Validate the recorder toolchain and remote backend
just demo-check

# Rebuild every public demo asset
just demo

# Rebuild a single scenario
just demo-one chat

# Record masters only
just demo-record dashboard

# Re-render assets from existing masters
just demo-render all

# Verify generated assets and README references
just demo-verify

# Drop staged master recordings
just demo-clean
```

Use `./scripts/demo.sh help` to see the full command surface and environment knobs.
