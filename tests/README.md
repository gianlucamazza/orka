# Test Layout

This directory is reserved for end-to-end and cross-crate integration tests.

- Prefer crate-local tests in `crates/<name>/tests` when the behavior belongs to a single crate.
- Use this directory only for scenarios that exercise multiple crates or the full runtime surface.
- Keep fixture data close to the test that owns it unless it is shared across multiple end-to-end suites.
