#!/usr/bin/env bash
# Bump Cargo.toml version, commit, tag, and push.
#
# Usage:
#   scripts/release.sh patch|minor|major   # bump from current Cargo.toml version
#   scripts/release.sh 1.2.3               # set explicit version
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "error: working tree not clean" >&2
    git status --short >&2
    exit 1
fi

current=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)
IFS='.' read -r major minor patch <<< "$current"

case "${1:-}" in
    major) next="$((major + 1)).0.0" ;;
    minor) next="${major}.$((minor + 1)).0" ;;
    patch) next="${major}.${minor}.$((patch + 1))" ;;
    "") echo "usage: $0 patch|minor|major|X.Y.Z" >&2; exit 1 ;;
    *)
        if [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            next="$1"
        else
            echo "error: invalid version '$1'" >&2
            exit 1
        fi
        ;;
esac

tag="v${next}"
if git rev-parse "$tag" >/dev/null 2>&1; then
    echo "error: tag $tag already exists" >&2
    exit 1
fi

sed -i.bak "0,/^version = \"${current}\"/s//version = \"${next}\"/" Cargo.toml
rm -f Cargo.toml.bak
cargo update --locked -p remux --precise "$next" 2>/dev/null || cargo update -p remux --precise "$next"

git add Cargo.toml Cargo.lock
git commit -m "release: v${next}"
git tag -a "$tag" -m "v${next}"

echo "Bumped ${current} -> ${next}, committed, tagged ${tag}."
echo "Push with: git push origin HEAD && git push origin ${tag}"
