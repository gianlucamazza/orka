# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| latest  | ✅        |

Orka follows a rolling-release model. Only the latest commit on `main` receives
security fixes. Always run the latest release.

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

**Email:** info@gianlucamazza.it

Alternatively, open a private GitHub Security Advisory:

1. Go to the repository on GitHub.
2. Click **Security** → **Advisories** → **New draft security advisory**.
3. Fill in the affected component, a description, and reproduction steps.

Please include:

- A description of the vulnerability
- Steps to reproduce
- Affected versions / components
- Potential impact

## Response Timeline

- **Acknowledgement:** within 48 hours
- **Initial assessment:** within 1 week
- **Fix or mitigation:** best effort, typically within 30 days for critical issues

## Scope

**In scope:**

- Remote code execution via the Orka API or adapters
- Authentication / authorisation bypasses (`orka-auth`)
- Secret exfiltration (AES-256-GCM encrypted secrets, API keys)
- SSRF / CSRF vulnerabilities in `orka-web` or the custom adapter
- Sandbox escapes in `orka-sandbox` (process or WASM)
- Prompt injection leading to privilege escalation via OS skills

**Out of scope:**

- Denial-of-service against a self-hosted instance you control
- Security of third-party LLM providers (Anthropic, OpenAI, Ollama, Ollama Cloud)
- Issues requiring physical access to the host machine

## Security Architecture

### Authentication

Orka supports two auth modes, configured under `[auth]`:

- **API key** — compared on every request; enable with `auth.enabled = true`
  and set `ORKA_API_KEY`.
- **JWT** — standard Bearer token verification (RS256 / HS256).

### Secret Encryption

Secrets stored via `orka secret set` are encrypted with AES-256-GCM before
being written to Redis. The 32-byte key is read from the env var named in
`secrets.encryption_key_env`. In `ORKA_ENV=production`, the server refuses to
start if the key is absent.

Generate a key: `openssl rand -hex 32`.

### SSRF Protection

The HTTP skill (`orka-web`) blocks requests to link-local and private ranges
by default. The deny-list is configured at `http.blocked_domains` and includes
`169.254.169.254` (AWS metadata). Extend it to block additional internal hosts.

### OS Skill Permission Model

| Level      | Config key                     | Default         |
| ---------- | ------------------------------ | --------------- |
| Permission | `os.permission_level`          | `read-only`     |
| Allow-list | `os.allowed_commands`          | `[]` (all)      |
| Deny-list  | `os.blocked_commands`          | hard-coded set  |
| Path allow | `os.allowed_paths`             | `/home`, `/tmp` |
| Path deny  | `os.blocked_paths`             | system paths    |
| Sudo       | `os.sudo.enabled`              | `false`         |
| Confirm    | `os.sudo.require_confirmation` | `true`          |

Sensitive environment variable names matching patterns in
`os.sensitive_env_patterns` are redacted from tool output.

### WASM Sandbox

WASM plugins run inside a Wasmtime sandbox with explicit `PluginCapabilities`
(env vars, filesystem paths, network). All capabilities are **deny-by-default**;
each plugin must be granted access individually in `plugins.capabilities.<name>`.

### Audit Log

Enable `audit.enabled = true` to write a JSONL audit trail of every skill
invocation to `audit.path` (default `orka-audit.jsonl`). Argument values are
hashed before logging to avoid leaking sensitive content.

## Production Hardening Checklist

- [ ] Set `ORKA_ENV=production` — enforces encryption key presence at startup.
- [ ] Generate and set a strong `ORKA_SECRET_ENCRYPTION_KEY` (32 bytes, hex).
- [ ] Enable `auth.enabled = true` and configure `ORKA_API_KEY`.
- [ ] Set `os.permission_level = "none"` unless OS skills are required.
- [ ] Keep `os.sudo.enabled = false` (default) in production.
- [ ] Review `http.blocked_domains` for your network topology.
- [ ] Enable `audit.enabled = true` for compliance and forensics.
- [ ] Restrict `plugins.capabilities` to the minimum required per plugin.
- [ ] Run behind a reverse proxy (nginx / Caddy) with TLS termination.
- [ ] Bind the server to `127.0.0.1` (default); never expose directly to internet.

## Disclosure

We ask that you do not publicly disclose the vulnerability until a fix has been
released. Reporters will be credited in the release notes unless they prefer to
remain anonymous.
