set -u
apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq strace >/dev/null 2>&1
cd /work/tmp/headtrace/cli
export GOFLAGS=-mod=mod
export GOCACHE=/work/tmp/headtrace/gocache
grep -c '^ok' /work/tmp/headtrace/run.out; grep -v '^ok\|no test files' /work/tmp/headtrace/run.out | head
timeout 1500 strace -f -qq --seccomp-bpf -e trace=execve,openat,open,newfstatat,statx,stat,lstat,readlink,chdir -e signal=none -s 300 \
  -o /work/tmp/headtrace/strace2.out go test -count=1 ./... >/work/tmp/headtrace/run2.out 2>&1
echo "exit $?"
grep -c . /work/tmp/headtrace/strace2.out
grep 'cli/\.git/HEAD"' /work/tmp/headtrace/strace2.out | grep -v ENOENT | awk '{print $1}' | sort -u > /work/tmp/headtrace/pids
wc -l < /work/tmp/headtrace/pids
for p in $(cat /work/tmp/headtrace/pids); do grep "^$p " /work/tmp/headtrace/strace2.out | grep -m1 execve | cut -c1-400; done | sort | uniq -c | sort -rn | head -40
