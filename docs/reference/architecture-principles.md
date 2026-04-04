# Orka Architecture Principles

This document is normative. It defines the architectural constraints that guide
new code, refactors, and future diagnostics for the Orka workspace.

Use [Architecture Diagram & Overview](architecture.md) for runtime flow. Use
this page for design constraints and review criteria.

## 1. Layered Workspace

The root [Cargo.toml](../../Cargo.toml) defines the
official workspace layer order. Internal crate dependencies must follow that
order.

- A crate may depend on crates in the same layer or lower layers.
- A crate must not depend on a crate declared in a higher layer.
- If a dependency feels necessary but points upward, that is a design smell to
  resolve, not a shortcut to normalize.

Default remediation order:

1. Move shared contracts and types to a lower layer.
2. Extract a lower-level trait or data model.
3. Move the depending crate to the appropriate higher layer if its real role has changed.

## 2. Single Responsibility Per Crate

Each crate should have one primary role.

- `core` and low-level crates should expose contracts, shared types, and narrow utilities.
- Configuration crates should own schema, loading, migrations, and validation; they should not become a second runtime composition root.
- Persistence crates should not know LLM-specific or orchestration-specific domain types unless those types are truly infrastructural.
- Orchestration crates may compose lower layers, but should avoid becoming a dumping ground for policy, storage, protocol mapping, and UI concerns together.

If a crate simultaneously owns domain policy, external protocol translation,
state persistence, and execution orchestration, it should be a split candidate.

## 3. Shared Types Belong Low

Cross-cutting types must live in the lowest semantically correct layer.

- A higher layer must not pull lower layers upward by forcing them to import orchestrator or provider-specific types.
- If multiple crates need the same type only for transport or serialization, move that type to a shared contract layer instead of duplicating upward knowledge.
- Prefer traits and neutral data models over reaching into a concrete service crate.

## 4. Size and Modularity

Large files are tolerated only when they are strongly cohesive.

- Data-centric files such as `types.rs` or narrow config modules may be large if they remain declarative.
- Logic-heavy modules above roughly 500 lines are presumed split candidates.
- Modules that mix IO, orchestration, retries, policy, and protocol translation in one place should be decomposed before new features are added.

Good split boundaries:

- protocol mapping
- persistence/store logic
- execution orchestration
- validation and policy
- rendering or presentation

## 5. Tests as a Structural Requirement

Every first-class crate should ship with a minimum safety baseline.

- New crates should start with real tests, not placeholders.
- Integration tests are preferred where a crate coordinates external boundaries.
- Inline unit tests are acceptable for focused pure logic, but they do not replace scenario coverage for orchestration crates.

The goal is not a vanity coverage number. The goal is to prevent architectural
surface area from growing without any executable contract.

## 6. Exceptions Must Be Explicit

Architectural exceptions should be rare and documented.

- Do not encode exceptions only in code.
- If a layering exception is temporarily accepted, record why it exists, the intended exit path, and the owning refactor.
- Prefer a short ADR or internal note over tribal knowledge.

## 7. Automation Expectations

`orka doctor` should gradually enforce these principles with conservative,
deterministic checks.

Initial automated signals:

- upward layering violations
- crates below a minimum test baseline
- oversized logic modules

These checks are detectors, not automatic refactoring tools. Their job is to
surface drift early enough that the design can still be corrected cheaply.
