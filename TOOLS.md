---
version: "0.2"
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
| Delegate a multi-step coding task autonomously   | `coding_delegate` |
| Store a durable fact for later retrieval         | `remember_fact` |
| Search remembered facts                          | `search_facts` |
| Ingest a whole document into the knowledge base  | `ingest_document`   |

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

## Memory (`remember_fact`, `search_facts`, `list_facts`, `forget_fact`, `ingest_document`, `list_documents`)

- `remember_fact` / `search_facts`: semantic fact memory with explicit persistence.
- `list_facts` / `forget_fact`: inspect and delete stored semantic facts.
- `ingest_document` / `list_documents`: full document ingestion pipeline (chunks + embeddings).

## Coding Delegate (`coding_delegate`)

- Use for complex, multi-step coding tasks: implementing features, fixing bugs, refactoring, or any
  task that requires reading files, making edits, and running commands autonomously.
- Orka selects the configured backend automatically (`claude_code` or `codex`); do not target the
  provider directly unless the configuration explicitly requires it.
- **Be imperative and specific**: describe _what to do_, not what to think about.
  Good: `"Add exponential backoff (max 3 retries) to fetch_data() in src/client.rs"`.
  Bad: `"Consider improving error handling"`.
- **Always include context**: mention relevant file paths, the language/framework, recent changes,
  or any architectural constraints. Use the `context` parameter for this.
- **Include verification**: pass a `verification` command (e.g. `cargo test -p crate-name`,
  `npm test`, `python -m pytest`) so the coding backend can confirm success before reporting done.
- **Scope narrowly**: one focused task per call. Split large changes into multiple sequential calls.
- **Do not micromanage steps**: the selected coding backend will decide how to implement — trust it
  to read files, choose the right approach, and follow project conventions on its own.
- **Override working directory**: use the `working_dir` parameter to run the delegated task in a specific
  directory (useful for monorepos or multi-project setups where the default cwd is not the target).

## Scheduler (`schedule_create`, `schedule_list`, `schedule_delete`)

- Create cron or one-shot tasks that invoke skills on a schedule.
