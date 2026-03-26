# Research Workflow

Orka's native `research` subsystem provides a structured loop for autonomous coding experiments. It is disabled by default and becomes available only when `[research].enabled = true`.

## Prerequisites

- Redis or Valkey available for normal Orka runtime state.
- `[os.coding].enabled = true` so campaign runs can call `coding_delegate`.
- `[scheduler].enabled = true` only if you want recurring runs from `campaign.cron`.
- A repository path that can be checked out into isolated git worktrees.

## Lifecycle

1. Create a campaign with the repository path, baseline ref, editable path allowlist, verification command, and target branch.
2. Start a run manually with `orka research campaign run <campaign-id>` or let the scheduler trigger it from `cron`.
3. Orka creates a dedicated git worktree and candidate branch for that run.
4. The coding task is delegated through `coding_delegate` inside that worktree.
5. Orka executes the configured verification command and stores the structured evaluation result.
6. If the candidate passes verification and improves the configured metric, it remains eligible for promotion.
7. `orka research candidate promote <candidate-id>` either promotes immediately or creates a promotion request, depending on policy.
8. Operators resolve pending approval with `orka research promotion approve <request-id>` or `orka research promotion reject <request-id>`.

## Campaign Creation

Example:

```bash
cargo run -p orka-cli -- research campaign create \
  --name "optimize-parser" \
  --workspace default \
  --repo-path /srv/repos/my-app \
  --baseline-ref main \
  --task "Reduce parse latency without changing behavior." \
  --verify "cargo test -p my-app && cargo bench -p my-app parser" \
  --editable-path src/parser \
  --metric-name latency_ms \
  --metric-regex 'latency_ms=([0-9.]+)' \
  --direction lower-is-better \
  --baseline-metric 42.0 \
  --min-improvement 1.0 \
  --target-branch research/optimize-parser
```

The verification command is mandatory. Metric extraction is optional; without it, a candidate is accepted on verification success alone.

## Promotion Requests

When `[research].require_promotion_approval = true`, or when the target branch matches one of `protected_target_branches`, `candidate promote` does not merge directly. It creates a `promotion request` instead.

That request is visible through:

```bash
cargo run -p orka-cli -- research promotion list
cargo run -p orka-cli -- research promotion show <request-id>
```

Resolution is explicit:

```bash
cargo run -p orka-cli -- research promotion approve <request-id>
cargo run -p orka-cli -- research promotion reject <request-id> --reason "benchmark variance too high"
```

When the checkpoint store is available, Orka also persists the approval pause as an interrupted checkpoint so the HITL state survives restart cleanly.

## Operational Notes

- `research` is bootstrap-gated. If the section is disabled, `/api/v1/research/*` returns service unavailable.
- Scheduler integration is optional. Manual `campaign run` works without `[scheduler]`.
- Promotion requests are a domain-specific lifecycle separate from generic run approval endpoints.
- Keep approval enabled for shared or protected branches.
