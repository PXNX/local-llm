#!/usr/bin/env bash
# New GUI release (semver): bumps gui/Cargo.toml (+ Cargo.lock), builds local-llm.exe locally with the release
# profile (clippy, tests, build), then commits, tags vX.Y.Z and pushes. The push to main starts
# .github/workflows/gui.yml, which builds again and publishes the GitHub release for the new version (any
# version bump pushed to main does, even without this script) - nothing is uploaded from here.
#   release.sh                 patch bump (0.1.0 -> 0.1.1)
#   release.sh minor|major     0.1.0 -> 0.2.0 | 1.0.0
#   release.sh 1.2.3[-rc.1]    exact version (a -suffix makes a GitHub pre-release)
set -euo pipefail
cd "$(dirname "$0")"
TOML=gui/Cargo.toml LOCK=gui/Cargo.lock
SEMVER='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$'

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "Releases are made from main."; exit 1; }
git diff --quiet "$TOML" "$LOCK" && git diff --cached --quiet || { echo "Commit or stash staged changes and edits to $TOML/$LOCK first."; exit 1; }
git fetch -q origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "main is not in sync with origin/main - pull/push first."; exit 1; }

cur=$(sed -n 's/^version = "\(.*\)"/\1/p' "$TOML" | head -n 1)
[[ $cur =~ $SEMVER ]] || { echo "$TOML has no semver version: $cur"; exit 1; }
ma=${BASH_REMATCH[1]} mi=${BASH_REMATCH[2]} pa=${BASH_REMATCH[3]} pre=${BASH_REMATCH[4]}
case "${1:-patch}" in
  major) new="$((ma + 1)).0.0" ;;
  minor) new="$ma.$((mi + 1)).0" ;;
  patch) [ -n "$pre" ] && new="$ma.$mi.$pa" || new="$ma.$mi.$((pa + 1))" ;;  # 1.2.0-rc.1 -> 1.2.0
  *) new="${1#v}" ;;
esac
[[ $new =~ $SEMVER ]] || { echo "Not a semver version: $new (usage: release.sh [patch|minor|major|X.Y.Z[-pre]])"; exit 1; }
[ "$new" != "$cur" ] || { echo "Already at $cur."; exit 1; }
git rev-parse -q --verify "refs/tags/v$new" >/dev/null && { echo "Tag v$new exists already."; exit 1; }

echo "=== Release v$new (was $cur)"
sed -i "0,/^version = \"$cur\"/s//version = \"$new\"/" "$TOML"
sed -i "/^name = \"local-llm-gui\"$/{n;s/^version = \".*\"/version = \"$new\"/}" "$LOCK"
revert() { git checkout -q -- "$TOML" "$LOCK"; echo "Local build failed - version bump reverted, nothing pushed."; exit 1; }

echo "=== Local release build (the same checks as GitHub Actions)"
(
  cd gui
  cargo clippy --release --locked -- -D warnings
  cargo test --release --locked
  cargo build --release --locked
) || revert
echo "Built gui/target/release/local-llm.exe (v$new)"

read -r -p "Commit, tag v$new and push (GitHub Actions publishes the release)? [y/N] " ok
[ "$ok" = y ] || [ "$ok" = Y ] || { git checkout -q -- "$TOML" "$LOCK"; echo "Cancelled - version bump reverted."; exit 1; }
git add "$TOML" "$LOCK"
git commit -q -m "Release v$new"
git tag -a "v$new" -m "v$new"
git push -q --atomic origin main "v$new"
echo "Pushed v$new - build and release: https://github.com/PXNX/local-llm/actions/workflows/gui.yml"
