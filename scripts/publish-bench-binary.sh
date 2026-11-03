#!/usr/bin/env bash
# Build `quod` from <ref> on GitHub Actions and publish it as the release
# bin-<shortsha> of the public bench repo, where its bench workflows download
# it. Light gh commands only; nothing compiles here. No tokens beyond your
# gh login.
#
#   scripts/publish-bench-binary.sh <ref>      (a branch, tag or commit of pango07/quod)
#
# Then bench it: gh workflow run bench-vitest.yml -R pango07/ci-bench -f bin=bin-<shortsha>
# (bin defaults to the latest release).
set -euo pipefail
ref=${1:?usage: publish-bench-binary.sh <ref>}
src=${QUOD_REPO:-pango07/quod}
dst=${BENCH_REPO:-pango07/ci-bench}
wf=bench-binary.yml

# The full commit, so the build is exactly what was asked for.
sha=$(gh api "repos/$src/commits/$ref" --jq .sha)
short=${sha:0:7}
tag=bin-$short
if gh release view "$tag" -R "$dst" >/dev/null 2>&1; then
  echo "$dst already has $tag ($sha)"; exit 0
fi

# The workflow runs from main once it is merged there, else from its branch.
if gh api "repos/$src/contents/.github/workflows/$wf?ref=main" >/dev/null 2>&1; then wf_ref=main; else wf_ref=ci/bench-binary; fi

since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run "$wf" -R "$src" --ref "$wf_ref" -f ref="$sha"
echo "dispatched $wf ($wf_ref) for $sha; waiting for the run"
id=
for _ in $(seq 1 30); do
  sleep 4
  id=$(gh run list -R "$src" --workflow "$wf" --event workflow_dispatch --limit 10 \
    --json databaseId,createdAt --jq "[.[] | select(.createdAt >= \"$since\")] | last | .databaseId // empty")
  [ -n "$id" ] && break
done
[ -n "$id" ] || { echo "no run appeared for $wf" >&2; exit 1; }
echo "run: https://github.com/$src/actions/runs/$id"
gh run watch "$id" -R "$src" --exit-status --interval 20 >/dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gh run download "$id" -R "$src" -n bench-binary -D "$tmp"
[ "$(cat "$tmp/sha.txt")" = "$sha" ] || { echo "artifact is for $(cat "$tmp/sha.txt"), not $sha" >&2; exit 1; }
gh release create "$tag" -R "$dst" --title "$tag" --notes "$short" "$tmp/quod-linux-x86_64.tar.gz"
echo "published https://github.com/$dst/releases/tag/$tag"
