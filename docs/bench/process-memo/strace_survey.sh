#!/bin/bash
# Which test binaries spawn subprocesses: every package's test binary once
# under strace -f execve-only, at the last commit of the history (the tree
# pm_history.py left in /work/pm/hist/cli).
set -u
rm -rf /work/pm/survey; mkdir -p /work/pm/survey; chmod 777 /work/pm/survey
printf '%s\n' '#!/bin/bash' 'out=/work/pm/survey/$(pwd | tr / _).trace' 'exec strace -f -qq --seccomp-bpf -e trace=execve -o "$out" -- "$@"' > /work/pm/strace_exec.sh
chmod 755 /work/pm/strace_exec.sh
cd /work/pm/hist/cli || exit 1
setpriv --reuid=1001 --regid=1001 --clear-groups env -i \
  CGO_ENABLED=0 GOTOOLCHAIN=local LANG=C.UTF-8 TZ=UTC PATH=/usr/local/go/bin:/usr/bin:/bin \
  GOMODCACHE=/work/hist/plain/gomod GOPATH=/work/hist/plain/gopath GOTELEMETRY=off GOPROXY=off GODEBUG=goindex=0 \
  GOCACHE=/work/pm/hist/gocache HOME=/work/pm/hist/home GOFLAGS="-mod=mod -count=1" \
  go test -exec /work/pm/strace_exec.sh ./... > /work/pm/survey.out 2>&1
echo "survey go test exit $?"
for f in /work/pm/survey/*.trace; do n=$(grep -c 'execve(' "$f"); echo "$n $(basename $f .trace)"; done | sort -rn > /work/pm/survey.txt
head -30 /work/pm/survey.txt
