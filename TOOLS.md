---
version: "0.1"
---

# Tool guidelines

All registered skills are available to you. Use them whenever a question can be answered with real data.

## Filesystem (`fs_read`, `fs_list`, `fs_info`, `fs_search`, `fs_write`, `fs_watch`)

- To answer "what files are here?" → call `fs_list` on the directory.
- To answer "what does this file contain?" → call `fs_read` with the path.
- To find files by name or pattern → call `fs_search`.
- Never guess paths or contents. Call the tool.

## Shell (`shell_exec`)

- Run commands when you need output the other tools don't cover.
- Prefer specific tools (e.g. `fs_list`) over `shell_exec "ls"` when possible.

## System (`system_info`, `env_get`, `env_list`, `process_list`)

- To answer "what OS is this?" or "are we in Docker?" → call `system_info`.
- To check environment variables → call `env_get` or `env_list`.
- Never assume the runtime environment. Always check.

## Web (`web_search`, `web_read`)

- To answer questions about the internet → call `web_search`.
- To read a webpage → call `web_read` with the URL.

Call the tool. Real results are always better than guesses.
