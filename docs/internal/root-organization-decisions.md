# Root Organization Decisions

**Date:** 2026-03-24
**Status:** Proposed

This note captures the next structural decisions required after the low-risk documentation cleanup of the repository root.

## Decision 1: Tool-Specific Root Files

### Scope

- `GEMINI.md`
- `.mcp.json`
- `.claude/`

### Current state

- These files are not part of the Rust product runtime.
- They support local agent or tool integrations.
- Some of them likely depend on conventional paths expected by external tools.
- `.mcp.json` explicitly references `tools/claude-channel/src/index.ts`.

### Options

#### Option A: Keep in root and document them as supported local integrations

Pros:
- Lowest risk.
- Preserves external tool conventions.
- Avoids breaking undocumented workflows.

Cons:
- Root remains slightly noisier.
- These files can be mistaken for product architecture.

#### Option B: Move them under a dedicated directory such as `meta/` or `tooling/`

Pros:
- Cleaner root.
- Stronger separation between product code and local integrations.

Cons:
- High probability of breaking external tool discovery.
- Requires per-tool validation and migration instructions.

### Recommendation

Choose **Option A** for now.

These files should remain in root until each consumer is explicitly validated. They should be treated as supported local integration artifacts, not as the default pattern for product code organization.

### Follow-up tasks

1. Add a short section in the root README explaining which root files are product/runtime and which are local integrations.
2. If desired later, evaluate each tool-specific file individually rather than moving them as a batch.

## Decision 2: Root Workspace Files vs Built-In Workspaces

### Scope

- `SOUL.md`
- `TOOLS.md`
- `workspaces/`

### Current state

- Root `SOUL.md` and `TOOLS.md` are discovered directly by the CLI and workspace loader.
- `workspaces/` is used by built-in runtime registration and by installation flow.
- Root files and built-in files are not semantically identical today:
  - root files are richer and repo-local
  - `workspaces/` contains distributable built-in workspace content

### Observed model

The repository currently has **two different workspace concepts**:

1. **Local repo workspace**
   Represented by root `SOUL.md` and `TOOLS.md`.
   Used for local discovery, local execution, and contributor tooling.

2. **Built-in distributable workspaces**
   Represented by files under `workspaces/`.
   Used by runtime registration and installation.

This distinction is real and currently useful, even if it is not fully explicit.

### Options

#### Option A: Preserve the two-tier model and document it clearly

Pros:
- Matches current runtime behavior.
- Lowest risk.
- Explains why files exist in both places without forcing premature unification.

Cons:
- Some duplication remains.
- Contributors must understand the distinction.

#### Option B: Unify root workspace files and `workspaces/`

Pros:
- Less duplication in theory.
- Cleaner conceptual model if done correctly.

Cons:
- High runtime risk.
- Requires changes across CLI discovery, workspace loader, installer, tests, and docs.
- Existing differences show these are not simple duplicates.

### Recommendation

Choose **Option A** for now.

The correct model is:

- root `SOUL.md` and `TOOLS.md` define the **local repository workspace**
- `workspaces/` stores **built-in runtime/distribution workspaces**

This should be documented as an architectural distinction, not treated as an accidental inconsistency.

### Follow-up tasks

1. Update docs to describe the two-tier workspace model explicitly.
2. Avoid deduplication or path refactors until there is a concrete runtime simplification plan.
3. If unification is ever desired, treat it as a dedicated refactor with compatibility requirements and tests.

## Decision 3: PKGBUILD Placement

### Scope

- `PKGBUILD`

### Current state

- `PKGBUILD` directly consumes `deploy/`, `orka.toml`, and release binaries.
- It looks like a first-class packaging asset, not an accidental file.

### Options

#### Option A: Keep it in root and document Arch packaging as supported

Pros:
- Conventional for Arch users.
- Zero workflow breakage.
- Clear if Arch packaging is considered first-class.

Cons:
- Adds one more specialized file to root.

#### Option B: Move it to `packaging/arch/`

Pros:
- Cleaner root.
- Better separation of distribution assets.

Cons:
- Changes Arch packaging workflow.
- Requires explicit packaging documentation updates.

### Recommendation

Choose **Option A** unless packaging assets are reorganized as a broader cross-platform initiative.

## Decision 4: Role of `tests/`

### Scope

- `tests/`

### Current state

- The directory is currently reserved but almost empty.
- Crate-local tests already live under individual crate directories.

### Options

#### Option A: Keep `tests/` reserved for future end-to-end and cross-crate suites

Pros:
- Matches common monorepo practice.
- Gives a clear home to future system-level scenarios.

Cons:
- Empty directories tend to drift without enforcement.

#### Option B: Remove it until the first real end-to-end suite exists

Pros:
- Slightly cleaner tree.
- No placeholder directories.

Cons:
- Small churn for little practical benefit.

### Recommendation

Choose **Option A**, but make it intentional.

The directory should either receive a first real end-to-end test in a future testing phase, or be revisited if it remains unused over time.

## Recommended Execution Order

1. Document the distinction between product/runtime root files and local tool integrations.
2. Document the two-tier workspace model in user-facing docs.
3. Keep `PKGBUILD` in root unless a broader packaging reorganization is approved.
4. Keep `tests/` reserved, then populate it when the first true end-to-end suite is defined.

## Non-Goals For Now

- Moving `SOUL.md`, `TOOLS.md`, or `orka.toml`
- Relocating `workspaces/`
- Batch-moving tool-specific integration files
- Deduplicating workspace content without runtime design work
