set -eu
cd /work/tmp/gi-mech/cli
git checkout -q -f --detach 9490d204a2ba
echo "== checkout index:"; GIT_TRACE2_PERF=1 git status --porcelain 2>&1 | grep -i -E 'refresh|cache_nr' | sed 's/^.*| //' | head
git read-tree HEAD
echo "== read-tree index:"; GIT_OPTIONAL_LOCKS=0 GIT_TRACE2_PERF=1 git status --porcelain 2>&1 | grep -i -E 'refresh|cache_nr' | sed 's/^.*| //' | head
which apt-get >/dev/null && (apt-get install -y -q strace >/dev/null 2>&1 && echo strace ok || echo "no strace install")
