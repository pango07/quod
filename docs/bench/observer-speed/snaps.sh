echo Y > /sys/module/fuse/parameters/enable_uring
for b in "$@"; do
  BIN=$b flock -w 3600 /work/tmp/heavy.lock timeout 3000 python3 /s/setup.py quod
done
