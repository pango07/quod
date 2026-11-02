#!/usr/bin/env bash
# Builds the demo history: for each upstream commit (oldest first) in
# ../mcommits.txt, a commit whose tree is upstream's minus .github/workflows
# and .github/dependabot.yml, plus ../demo/workflows/*.yml and
# ../demo/QUOD_DEMO.md. Writes "<demo sha> <upstream sha>" lines to
# ../mmirror.txt and points refs/heads/demo at the last one.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here/moto"
export GIT_INDEX_FILE="$here/moto/.git/mirror-index"
prev=
: > "$here/mmirror.txt"
add() { # path file
  local b
  b=$(git hash-object -w "$2")
  git update-index --add --cacheinfo "100644,$b,$1"
}
while read -r up; do
  rm -f "$GIT_INDEX_FILE"
  git read-tree "$up"
  git rm -r -q --cached --ignore-unmatch .github/workflows .github/dependabot.yml >/dev/null
  for f in "$here"/mdemo/workflows/*.yml; do
    add ".github/workflows/$(basename "$f")" "$f"
  done
  add QUOD_DEMO.md "$here/mdemo/QUOD_DEMO.md"
  tree=$(git write-tree)
  subject=$(git show -s --format=%s "$up")
  msg=$(printf '%s\n\nUpstream: getmoto/moto@%s\nMirrored for the Quod CI benchmark: upstream tree with .github/workflows replaced (see QUOD_DEMO.md).\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n' "$subject" "$up")
  export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE
  GIT_AUTHOR_NAME=$(git show -s --format=%an "$up")
  GIT_AUTHOR_EMAIL=$(git show -s --format=%ae "$up")
  GIT_AUTHOR_DATE=$(git show -s --format=%aI "$up")
  export GIT_COMMITTER_NAME=pango07 GIT_COMMITTER_EMAIL=juliovaught@yahoo.com
  if [ -z "$prev" ]; then
    c=$(echo "$msg" | git commit-tree "$tree")
  else
    c=$(echo "$msg" | git commit-tree "$tree" -p "$prev")
  fi
  echo "$c $up" >> "$here/mmirror.txt"
  prev=$c
done < "$here/mcommits.txt"
git update-ref refs/heads/demo "$prev"
rm -f "$GIT_INDEX_FILE"
wc -l < "$here/mmirror.txt"
