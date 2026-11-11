# Working in this repository

Same rules as `CLAUDE.md`, spelled out for every agent (OpenCode, Codex, Pi, Claude). Keep the two in sync.

## Concurrent sessions

Other agent sessions may be working in this same checkout at the same time and may edit the same files you are editing.

- Only commit your own changes. Stage files (or hunks) explicitly, e.g. `git add <file>` or `git add -p`, never `git add -A` / `git add .` / `git commit -a`.
- Before committing, review `git diff --staged` and make sure every staged line is yours. If a file contains both your edits and someone else's, stage only your hunks.
- Do not revert, reformat, stash, or "clean up" changes you did not make (no `git stash`, `git checkout -- <file>`, `git restore`, or `git reset --hard` on files touched by others).
- If an unexpected change blocks your work (e.g. breaks the build), mention it to the user instead of fixing or discarding it yourself.

## Where things live

| Area | Path | Notes |
|---|---|---|
| Models (all tools) | `models/` = `MODELS_DIR` | ComfyUI checkpoints directly inside; `ollama/`, `huggingface/`, `torch/`, `u2net/`, `kokoro/` for the other tools. Resolve it with `common/storage.py`, `common/models-env.bat` or `gui/src/paths.rs`. |
| Model manifest | `models.json` | Model list for the GUI, also embedded into the exe. |
| GUI | `gui/` (Rust/egui) | Pages in `gui/src/app/`, one module per flow in `gui/src/flows/`, servers in `servers.rs`, llama.cpp router in `llamacpp.rs`, model store in `models/`. |
| Flow scripts | `img2video/`, `stickers/`, `comedy/`, `characters/`, `caricatures/`, `vectorize/`, `soundfx/`, `topvideos/` | Python, run with ComfyUI's embedded Python. The `*_workflow_api.json` files are ComfyUI graphs. |
| Shared Python | `common/` | `storage.py` (paths, routes HF/torch/rembg), `console_run.py`, `llm.py`, `log.py`. |
| Agent configs | `config/` | `llama-models.ini`, `opencode*.json`, `pi-*.{md,json,ts}`, `*.Modelfile`. |
| Entry points | `*.bat` in the root | `0-download-all.bat` … `12-comedy.bat`, `start-comfyui.bat`, `setup.bat`, `release.bat`. |
| ComfyUI | `ComfyUI_windows_portable/` | Local install, not tracked in git. |
| Release CI | `.github/workflows/gui.yml` | Builds and publishes on every new `gui/Cargo.toml` version on `main`. |

Conventions:

- Every model download goes into `MODELS_DIR`. Never download into another place; the folders models used to live in are listed in `paths::old_stores` so the GUI can offer to move them.
- Python is only ever `ComfyUI_windows_portable/python_embeded/python.exe`, there is no system Python.
- Check GUI work with `cargo clippy --release --locked -- -D warnings` and `cargo test --release --locked` from `gui/`.
- New flow = Python script + a `.bat` wrapper + a page in `gui/src/flows/`, plus an entry in `models.json` when models are involved.
- Releases go through `release.bat [patch|minor|major|X.Y.Z[-pre]]`; never upload a GitHub release by hand.
- Line endings follow `.gitattributes` (`.bat` CRLF, everything else LF).

## Committing

- Commit your own changes yourself, feature by feature: as soon as one self-contained piece of work is done (a new flow, a GUI page, a wired-up `.bat`), commit it before starting the next. Do not collect a whole task into one big commit at the end, and do not leave finished work uncommitted.
- Each commit should build on its own and have a message that says what that feature does, with the area as prefix like the existing history: `GUI:`, `Comedy:`, `Coding:`, `CI:`.
- Author and committer are `PXNX <loool1912@web.de>` (`git config --local user.name/user.email`). Never let another identity leak into history; if a commit shows the wrong author, rewrite it instead of leaving it.
- The rules from "Concurrent sessions" still apply to every one of these commits.
