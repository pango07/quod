set -eu
apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq strace >/dev/null 2>&1
which strace
rm -rf /work/tmp/headtrace && mkdir -p /work/tmp/headtrace
cd /work/tmp/headtrace
git clone -q --no-hardlinks /work/cli cli
cd cli
git checkout -q -b pr
export GOFLAGS=-mod=mod
export GOCACHE=/work/tmp/headtrace/gocache
# Warm compile (no trace), then trace a run.
timeout 900 go test ./... >/work/tmp/headtrace/warm.out 2>&1 || true
echo warm done
timeout 1200 strace -f -qq --seccomp-bpf -e trace=execve,openat,open,newfstatat,statx,stat,lstat,readlink -e signal=none -s 256 \
  -o /work/tmp/headtrace/strace.out go test ./... >/work/tmp/headtrace/run.out 2>&1 || true
echo traced
grep -c . /work/tmp/headtrace/strace.out
grep '\.git/HEAD\|"HEAD"' /work/tmp/headtrace/strace.out | head -50
