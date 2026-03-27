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

## Web (`web_search`, `web_read`)

- Search results already include full page content inline — do NOT call `web_read` after `web_search`.
- Make ONE search call, then answer directly.

## Filesystem

- List → `fs_list`. Read → `fs_read`. Find by name or pattern → `fs_search`.

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
  directory (useful for monorepos or multi-project setups).

## Memory (`remember_fact`, `search_facts`, `list_facts`, `forget_fact`, `ingest_document`, `list_documents`)

- `remember_fact` / `search_facts`: semantic fact memory with explicit persistence.
- `list_facts` / `forget_fact`: inspect and delete stored semantic facts.
- `ingest_document` / `list_documents`: full document ingestion pipeline (chunks + embeddings).
