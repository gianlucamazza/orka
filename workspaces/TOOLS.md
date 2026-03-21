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
| Store a finding for later retrieval              | `memory_store` |
| Ingest a whole document into the knowledge base  | `doc_ingest`   |

## Web (`web_search`, `web_read`)

- Search results already include full page content inline — do NOT call `web_read` after `web_search`.
- Make ONE search call, then answer directly.

## Filesystem

- List → `fs_list`. Read → `fs_read`. Find by name or pattern → `fs_search`.

## Knowledge (`memory_store`, `memory_search`, `doc_ingest`, `doc_list`)

- `memory_store` / `memory_search`: semantic store for facts and findings.
- `doc_ingest` / `doc_list`: full document ingestion pipeline (chunks + embeddings).
