apt-get update -qq >/dev/null 2>&1; apt-get install -y -qq linux-perf >/dev/null 2>&1
for n in "$@"; do
  echo "== $n"
  perf report -i /work/obsspeed/prof/$n.data -n --no-children --sort comm --stdio -g none 2>/dev/null | grep -v '^#' | grep -v '^$' | head -8
  echo "distinct pids per comm:"
  perf script -i /work/obsspeed/prof/$n.data -F comm,pid 2>/dev/null | sort -u | awk '{print $1}' | sort | uniq -c | sort -rn | head -6
done
