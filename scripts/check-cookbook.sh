#!/usr/bin/env bash
# Check the cookbook's layout (cookbook/CONTRIBUTING.md): the recipe directories against the index rows in both
# directions, kebab-case names, each README's six headings and Author line, and a shebang on every script. The
# recipes' shellcheck and node runs are in scripts/lint.sh.
#
# Usage: scripts/check-cookbook.sh
set -euo pipefail

cd "$(dirname "$0")/.."
(($# == 0)) || { echo "usage: scripts/check-cookbook.sh" >&2; exit 2; }
fail=0
error() { echo "cookbook: $*" >&2; fail=1; }

dirs=$(find cookbook -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort)
# links only from table rows, so a link in prose or a fenced example cannot satisfy the check
links=$(grep -E '^\|' cookbook/README.md |
  grep -oE '\]\([a-z0-9][a-z0-9-]*/\)' |
  sed -e 's/^](//' -e 's|/)$||' | sort -u || true)
for name in $(comm -23 <(printf '%s\n' "$dirs") <(printf '%s\n' "$links")); do
  error "$name is missing from the index"
done
for name in $(comm -13 <(printf '%s\n' "$dirs") <(printf '%s\n' "$links")); do
  error "index row '$name' points at no such directory"
done

for dir in cookbook/*/; do
  name=$(basename "$dir")
  if printf '%s' "$name" | grep -qE '[^a-z0-9-]|--|^-|-$'; then
    error "$name is not kebab-case"
  fi
  readme="$dir/README.md"
  if [ ! -f "$readme" ]; then
    error "$name has no README.md"
    continue
  fi
  for heading in "## What it does" "## Requirements" "## Setup" "## Usage" "## How it works" "## Limits"; do
    grep -qxF "$heading" "$readme" || error "$name README is missing the '$heading' heading"
  done
  if ! grep -qE '^Author: \[@[A-Za-z0-9-]+\]\(https://github\.com/[A-Za-z0-9-]+\)' "$readme"; then
    error "$name README has no 'Author: [@login](https://github.com/login)' line"
  fi
done

while IFS= read -r -d '' script; do
  head -c 2 "$script" | grep -q '#!' || error "$script has no shebang"
done < <(find cookbook -type f \( -name '*.sh' -o -name '*.test.js' \) -print0)

exit "$fail"
