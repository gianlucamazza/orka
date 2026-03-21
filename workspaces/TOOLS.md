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

## Web (`web_search`, `web_read`)

- Search results already include full page content inline — do NOT call `web_read` after `web_search`.
- Make ONE search call, then answer directly.

## Filesystem

- List → `fs_list`. Read → `fs_read`. Find by name or pattern → `fs_search`.

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
  directory (useful for monorepos or multi-project setups).

## Knowledge (`memory_store`, `memory_search`, `doc_ingest`, `doc_list`)

- `memory_store` / `memory_search`: semantic store for facts and findings.
- `doc_ingest` / `doc_list`: full document ingestion pipeline (chunks + embeddings).
