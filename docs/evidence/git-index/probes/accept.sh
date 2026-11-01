#!/usr/bin/env bash
# CLI acceptance on cli/cli under the shared lock.
cd /src
flock /work/tmp/heavy.lock bash -c 'scripts/acceptance.sh go > /work/tmp/gidx-acceptance-go.log 2>&1; echo "acceptance exit $?" >> /work/tmp/gidx-acceptance-go.log'
grep -E "^(A[0-9]|W|T[0-9]|manifest:|==)|panicked|FAILED|test result|false hits|exit" /work/tmp/gidx-acceptance-go.log | cut -c1-260
