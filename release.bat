@echo off
rem New GUI release (semver): bumps gui\Cargo.toml, builds local-llm.exe locally (release profile), tags vX.Y.Z
rem and pushes - GitHub Actions then builds and publishes the release.
rem   release.bat [patch|minor|major|X.Y.Z[-pre]]   (default patch)
"%ProgramFiles%\Git\bin\bash.exe" "%~dp0release.sh" %*
pause
