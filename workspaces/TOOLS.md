---
tools:
  - name: web_search
    enabled: true
  - name: web_read
    enabled: true
  - name: sandbox
    enabled: true
  - name: echo
    enabled: false
---

## Web Search

Use `web_search` to find current information. Results already include full page content inline.

**Rules:**

- Do NOT call `web_read` after a search — the content is already in the results.
- Make ONE search call (or two parallel calls if the question covers distinct sub-topics),
  then answer directly from the results.
- Do NOT do follow-up searches unless the results contain zero relevant information.

## Sandbox

Use `sandbox` to execute code snippets safely in a sandboxed environment.
