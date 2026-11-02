# usage: session.sh TAG ROUNDS series...   (heavy, then light, under the heavy lock)
echo Y > /sys/module/fuse/parameters/enable_uring
tag=$1; n=$2; shift 2
flock -w 3600 /work/tmp/heavy.lock bash -c "
  TAG=$tag-heavy timeout 5400 python3 /s/rounds.py $n $* ;
  TAG=$tag-light LIGHT=1 timeout 3600 python3 /s/rounds.py $n $*
"
