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
  - name: system_info
    enabled: true
  - name: shell_exec
    enabled: true
  - name: fs_read
    enabled: true
  - name: fs_list
    enabled: true
  - name: process_list
    enabled: true
  - name: http_request
    enabled: true
  - name: memory_store
    enabled: true
  - name: memory_search
    enabled: true
  - name: doc_ingest
    enabled: true
  - name: doc_list
    enabled: true
  - name: schedule_create
    enabled: true
  - name: schedule_list
    enabled: true
  - name: schedule_delete
    enabled: true
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

## HTTP Client

Use `http_request` for calling external APIs and services. Supports GET, POST, PUT, PATCH, DELETE, HEAD methods with custom headers, body, and bearer auth.

## Knowledge & RAG

- `memory_store`: Save content with semantic embedding for later retrieval.
- `memory_search`: Search for semantically similar content.
- `doc_ingest`: Ingest documents (PDF, HTML, MD, TXT) by parsing, chunking, and embedding.
- `doc_list`: List ingested documents in a collection.

## Scheduler

- `schedule_create`: Create cron or one-shot scheduled tasks.
- `schedule_list`: List active scheduled tasks.
- `schedule_delete`: Remove a scheduled task by ID or name.
