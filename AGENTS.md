# AGENTS.md

All work related to the Janex file format must follow `docs/spec/FileFormat.md`.

- Treat `docs/spec/FileFormat.md` as the only source of truth for the file format.
- Do not restate format details in `AGENTS.md`; read `docs/spec/FileFormat.md` directly when needed.
- If implementation and spec diverge, update the implementation to match the spec unless the task explicitly changes the spec.
- If a task changes the file format itself, update `docs/spec/FileFormat.md` first, then update code and tests.
