---
version: "0.1"
---

# Tool guidelines

## Choosing between overlapping tools

| Goal                                             | Use            |
| ------------------------------------------------ | -------------- |
| Find information on the internet                 | `web_search`   |
| Read a known URL                                 | `web_read`     |
| Call an external API with headers, body, or auth | `http_request` |
| Run isolated code safely                         | `sandbox`      |
| Run system commands or scripts                   | `shell_exec`   |
| Delegate a multi-step coding task autonomously   | `claude_code`  |
| Store a finding for later retrieval              | `memory_store` |
| Ingest a whole document into the knowledge base  | `doc_ingest`   |

## Filesystem (`fs_read`, `fs_list`, `fs_info`, `fs_search`, `fs_write`, `fs_watch`)

- List → `fs_list`. Read → `fs_read`. Find by name or pattern → `fs_search`.
- Prefer these over `shell_exec "ls"` or `shell_exec "cat"`.

## Shell (`shell_exec`)

- Use for anything not covered by a dedicated tool.
- Does not interpret shell syntax — pass commands as argument arrays when possible.

## System (`system_info`, `env_get`, `env_list`, `process_list`, `process_info`, `process_signal`)

- OS and environment questions → `system_info` or `env_get` / `env_list`. Never assume.

## Web (`web_search`, `web_read`)

- Search results already include full page content inline — do NOT call `web_read` after `web_search`.
- Make ONE search call, then answer directly. Search again only if results contain zero relevant information.

## HTTP (`http_request`)

- For APIs that need custom headers, POST bodies, or bearer auth. Not for general browsing.

## Code (`sandbox`)

- Supported: python, bash, wasm. Use for isolated or untrusted code execution.

## Knowledge (`memory_store`, `memory_search`, `doc_ingest`, `doc_list`)

- `memory_store` / `memory_search`: semantic store for facts and findings.
- `doc_ingest` / `doc_list`: full document ingestion pipeline (chunks + embeddings).

## Claude Code (`claude_code`)

- Use for complex, multi-step coding tasks: implementing features, fixing bugs, refactoring, or any
  task that requires reading files, making edits, and running commands autonomously.
- **Be imperative and specific**: describe _what to do_, not what to think about.
  Good: `"Add exponential backoff (max 3 retries) to fetch_data() in src/client.rs"`.
  Bad: `"Consider improving error handling"`.
- **Always include context**: mention relevant file paths, the language/framework, recent changes,
  or any architectural constraints. Use the `context` parameter for this.
- **Include verification**: pass a `verification` command (e.g. `cargo test -p crate-name`,
  `npm test`, `python -m pytest`) so Claude Code can confirm success before reporting done.
- **Scope narrowly**: one focused task per call. Split large changes into multiple sequential calls.
- **Do not micromanage steps**: Claude Code will decide how to implement — trust it to read files,
  choose the right approach, and follow project conventions on its own.
- **Override working directory**: use the `working_dir` parameter to run Claude Code in a specific
  directory (useful for monorepos or multi-project setups where the default cwd is not the target).

## Scheduler (`schedule_create`, `schedule_list`, `schedule_delete`)

- Create cron or one-shot tasks that invoke skills on a schedule.
