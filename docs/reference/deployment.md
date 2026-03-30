# Deployment Guide

## Prerequisites

| Dependency | Version | Purpose                      |
| ---------- | ------- | ---------------------------- |
| Redis      | 7+      | Bus, queue, session, memory  |
| Qdrant     | 1.14+   | RAG vector store (optional)  |
| Docker     | 24+     | Container runtime (optional) |
| Rust       | 1.91+   | Build from source            |

Valkey (Redis-compatible) is also supported as a drop-in Redis replacement.

---

## Docker Compose (recommended)

```bash
cp .env.example .env
# Fill in required values: ANTHROPIC_API_KEY, MOONSHOT_API_KEY, ORKA_SECRET_ENCRYPTION_KEY, etc.

docker compose up -d
```

Services started:

| Service     | Default port | Notes                   |
| ----------- | ------------ | ----------------------- |
| redis       | 6379         | Internal only           |
| qdrant      | 6334         | Internal only (gRPC)    |
| orka-server | 8080, 8081   | Health + custom adapter |

The Compose file uses `cargo-chef` for layer caching; rebuild time after code
changes is typically < 30 s.

---

## Manual / Bare-Metal

```bash
# 1. Start Redis
docker run -d --name redis -p 6379:6379 redis:7-alpine

# 2. (Optional) Start Qdrant
docker run -d --name qdrant -p 6334:6334 qdrant/qdrant:v1.14.0

# 3. Build release binary
cargo build --release

# 4. Configure
$EDITOR orka.toml

# 5. Run
ORKA_ENV=production \
ORKA_SECRET_ENCRYPTION_KEY=$(openssl rand -hex 32) \
./target/release/orka-server
```

---

## Native Linux Installation (systemd)

```bash
# Install common dev dependencies, start Redis/Valkey, and verify the build
just setup

# Build and install the systemd service
just install
systemctl enable --now orka-server

# Uninstall (preserves config and data)
just uninstall
```

The repository follows a portable-first distribution model:

- **Portable upstream artifacts**: OCI image and release tarball work across distributions.
- **Generic systemd install**: `scripts/install.sh` installs the service using FHS paths and discovers the host's systemd directories.
- **Native packaging**: Arch packaging is currently provided via `PKGBUILD`; additional distro-native packages can build on the same `deploy/` assets.

### Native Install Bundle

For script-driven installs that do not build on the target host, create a
portable install bundle that preserves the repository layout expected by
`scripts/install.sh`:

```bash
cargo build --release --bin orka-server --bin orka
./scripts/create-install-bundle.sh \
  --profile release \
  --output-dir dist/orka-install-bundle-amd64 \
  --tarball dist/orka-install-bundle-amd64.tar.gz
```

The resulting bundle contains:

- the prebuilt `orka-server` and `orka` binaries under `target/release/`
- `scripts/install.sh`
- shared `deploy/` assets
- `workspaces/`
- desktop/icon assets
- the canonical `orka.toml`

This keeps local/manual installs and remote installs on the same script path.

### Remote Native Reinstall

To reinstall a remote native service from a prebuilt bundle:

```bash
./scripts/deploy-remote-native.sh \
  --bundle-tarball dist/orka-install-bundle-amd64.tar.gz \
  --host example-host \
  --user deploy \
  --port 22 \
  --release-name "$(git rev-parse --short HEAD)"
```

The remote script upload extracts the bundle under `~/orka-deploy/releases/`
and runs `sudo ./scripts/install.sh --force --yes` from there. The target host
does not need Rust or Cargo installed.

### Drone Homelab Flow

The homelab Drone flow is designed to complement, not replace, GitHub Actions:

- GitHub Actions remain the public CI and GitHub release path.
- Drone handles homelab publication and native remote reinstall.
- The OCI image published to the registry and the native install bundle should
  always come from the same commit SHA, but the native reinstall should use the
  bundle rather than extracting binaries from the container image.

`just setup` currently supports common `pacman`, `apt`, and `dnf` based development environments.
It expects a `rustup`-managed Rust toolchain that satisfies the workspace
minimum (`rust-version = 1.91`). On distributions with older distro-provided
`rustc`/`cargo`, install or update Rust via `rustup` before running `just setup`.

### Distribution Support Matrix

| Distribution family | Portable install | Native package status | Source |
| ------------------- | ---------------- | --------------------- | ------ |
| Arch Linux          | Yes              | Implemented           | `PKGBUILD` |
| Debian / Ubuntu     | Yes              | Scaffolded            | `packaging/debian/` with Rust >= 1.91 |
| Fedora / RHEL       | Yes              | Scaffolded            | `packaging/fedora/` with Rust >= 1.91 |

The native package scaffolds are intentionally thin and reuse the shared assets in `deploy/`. This keeps service hardening, filesystem layout, and runtime identity consistent across distributions.

The `just install` target:

1. Builds `orka-server` in release mode.
2. Writes `/etc/orka/orka.toml` (template).
3. Installs `orka-server.service` using the selected binary prefix.
4. Installs `sysusers.d` and `tmpfiles.d` definitions.
5. Adds a `sudoers` entry for OS skills if `os.sudo.allowed = true`.

The repository keeps the canonical example configuration at the root as
`orka.toml`; there is no separate `orka.toml.example` file.

Best practice for distro-native packages:

- Keep configuration in `/etc/orka`.
- Keep mutable state in `/var/lib/orka`.
- Install systemd units in the distro's system unit directory rather than hardcoding a single path.
- Use `sysusers.d` and `tmpfiles.d` for service users and state directories where supported.
- Avoid hardcoding distro-specific service dependencies in upstream service units unless they are guaranteed by the package.

---

## Environment Variables

| Variable                     | Required in prod | Description                                       |
| ---------------------------- | ---------------- | ------------------------------------------------- |
| `ORKA_ENV`                   | yes              | Set to `production` to enforce encryption key     |
| `ORKA_SECRET_ENCRYPTION_KEY` | yes              | 32-byte hex key for AES-256-GCM secret encryption |
| `ORKA_CONFIG`                | no               | Path to config file (default: `./orka.toml`)      |
| `ANTHROPIC_API_KEY`          | if using Claude  | Anthropic provider fallback                       |
| `MOONSHOT_API_KEY`           | if using Moonshot | Moonshot provider fallback                       |
| `OPENAI_API_KEY`             | if using OpenAI  | OpenAI provider fallback                          |
| `ORKA_API_KEY`               | recommended      | API key for authenticated requests                |

Generate a secure encryption key:

```bash
openssl rand -hex 32
```

---

## Reverse Proxy (nginx)

```nginx
server {
    listen 443 ssl;
    server_name orka.example.com;

    ssl_certificate     /etc/letsencrypt/live/orka.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/orka.example.com/privkey.pem;

    # Health / API
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }

    # Custom adapter + WebSocket
    location /api/v1/ {
        proxy_pass http://127.0.0.1:8081;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

---

## Health Probes (Kubernetes)

```yaml
livenessProbe:
  httpGet:
    path: /health/live
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /health/ready
    port: 8080
  initialDelaySeconds: 10
  periodSeconds: 5
```

---

## Observability

### Prometheus

Set `observe.backend = "prometheus"` to expose `/metrics` in Prometheus text format.

```toml
[observe]
backend = "prometheus"
```

Scrape config:

```yaml
- job_name: orka
  static_configs:
    - targets: ["orka.example.com:8080"]
  metrics_path: /metrics
```

Key metrics:

| Metric                          | Type      | Description                   |
| ------------------------------- | --------- | ----------------------------- |
| `orka_messages_received_total`  | counter   | Inbound messages by channel   |
| `orka_llm_completions_total`    | counter   | LLM API calls                 |
| `orka_skill_invocations_total`  | counter   | Skill invocations by name     |
| `orka_errors_total`             | counter   | Errors by source              |
| `orka_llm_input_tokens_total`   | counter   | Total input tokens consumed   |
| `orka_llm_output_tokens_total`  | counter   | Total output tokens generated |
| `orka_llm_cost_dollars_total`   | counter   | Estimated LLM cost (USD)      |
| `orka_handler_duration_seconds` | histogram | End-to-end handler latency    |
| `orka_llm_duration_seconds`     | histogram | LLM call latency              |

### OpenTelemetry

```toml
[observe]
backend = "otlp"
otlp_endpoint = "http://otel-collector:4317"
```

### Audit Log

```toml
[audit]
enabled = true
output  = "file"            # or "redis"
path    = "/var/lib/orka/audit.jsonl"
```

Each line is a JSON record:

```json
{"timestamp_ms":1711000000000,"event":"skill_invoked","skill":"shell_exec","message_id":"...","caller_id":"user-42","args_hash":"len34:chk00a3b4c5d6e7f8a9"}
{"timestamp_ms":1711000000250,"event":"skill_completed","skill":"shell_exec","message_id":"...","duration_ms":250,"success":true}
```

### TUI Dashboard

```bash
orka dashboard --interval 2   # refresh every 2 seconds
```

Displays: health, uptime, worker count, queue depth, dependency readiness,
Prometheus metrics, active sessions, and DLQ depth. Press `r` to force-refresh,
`q` / `Esc` to quit.

---

## Resource Sizing

| Load profile | Redis RAM | Workers | Notes                        |
| ------------ | --------- | ------- | ---------------------------- |
| Dev / hobby  | 256 MB    | 2       | In-memory fallback works too |
| Small team   | 512 MB    | 4       | Default config               |
| Production   | 2 GB+     | 8–16    | Tune `worker.concurrency`    |

LLM latency dominates: workers spend most of their time waiting on the LLM API.
Increase `worker.concurrency` freely — each worker consumes < 10 MB of RAM.

---

## Upgrading

```bash
# Validate new config without applying
orka config migrate --dry-run

# Apply schema migration
orka config migrate
```

Check the release notes for breaking config changes before upgrading.
