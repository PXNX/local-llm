# CLAUDE.md

## Models for agents

- **Orchestrator: Opus 5.5** (`claude-opus-5-5`). The main session plans, splits the work, makes the design
  decisions, reviews what comes back and writes the final changes where judgment matters.
- **Subagents: Haiku 5.5** (`claude-haiku-5-5`). Spawn every execution helper with `model: "haiku"`: searches,
  file sweeps, mechanical edits, running builds/tests/linters, testers and verifiers. Give each one a narrow,
  self-contained task and ask for a short result.
- Only fall back to Opus for a subagent when Haiku has failed at the task or it needs real design judgment.

## Repo essentials

- **One models folder.** Every model lives in `MODELS_DIR` (`.env`; relative = inside the repo; default
  `models/`). ComfyUI folders sit directly in it, other tools in subfolders: `ollama/`, `huggingface/`,
  `torch/`, `u2net/`, `kokoro/`. Never download into another place. Resolve it with `common/storage.py`
  (Python, `import common` routes HF/torch/rembg), `common/models-env.bat` (.bat) or `gui/src/paths.rs` (GUI).
  Folders models used to be in are listed in `paths::old_stores` so the GUI offers to move them.
- `models.json` is the model manifest for the GUI (also embedded into the exe).
- GUI: Rust/egui in `gui/`. Check with `cargo clippy --release --locked -- -D warnings` and
  `cargo test --release --locked` from `gui/`.
- Flows are Python scripts run with ComfyUI's embedded Python
  (`ComfyUI_windows_portable/python_embeded/python.exe`); there is no system Python.
- Releases (semver): `release.bat [patch|minor|major|X.Y.Z[-pre]]` bumps `gui/Cargo.toml`, builds the exe
  locally with the release profile (clippy, tests, build), then tags `vX.Y.Z` and pushes. Only
  `.github/workflows/gui.yml` publishes the GitHub release: every push to main whose `gui/Cargo.toml` version
  has no release yet gets one (tagged there if needed) - never upload one by hand.
- Line endings follow `.gitattributes` (`.bat` CRLF, everything else LF).
