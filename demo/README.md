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

The demo pipeline assumes a live Orka backend. By default it targets:

- `ORKA_DEMO_USE_SSH_TUNNEL=1`
- `ORKA_DEMO_SSH_HOST=odroid`
- remote Orka ports `18080` and `18081`

With the default tunnel mode, the recorder talks to a temporary local forward and does not rely on `orka-odroid` resolving correctly on the recorder machine.

Disable the tunnel only if the backend is directly reachable and then set `ORKA_DEMO_SERVER_URL` and `ORKA_DEMO_ADAPTER_URL` explicitly. Set `ORKA_DEMO_API_KEY` if the target backend is protected. The script also honors `ORKA_DEMO_ORKA_BIN` if you want to pin a specific `orka` CLI binary.

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
