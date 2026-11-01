#!/usr/bin/env bash
# What a step sees for stat(2) of served workspace files (quod run, CLI path).
set -e
P=/work/tmp/gi-probe
rm -rf $P /work/tmp/gi-state
mkdir -p $P/dir
cd $P
git init -q
echo hello > a; echo world > b; echo x > dir/c
git add -A
git -c user.email=x@x -c user.name=x commit -qm init
/src/target/release/quod --state /work/tmp/gi-state run --repo $P --toolchain go -- sh -c \
  'stat -c "%n ino=%i dev=%d uid=%u gid=%g mtime=%.9Y ctime=%.9Z size=%s mode=%f" a b dir dir/c . ; id; grep workspace /proc/self/mountinfo' 2>&1 | tail -20
