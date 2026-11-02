cd /work/tmp/headtrace
# parent of each pid: the clone/vfork line "... = <pid>" in another pid's trace is not traced (no clone in -e). Use cwd via chdir? Instead: list per pid all .git paths touched.
for p in $(cat pids); do
  args=$(grep -E "^$p +execve" strace2.out | tail -1 | sed 's/.*\["\/usr\/bin\/git", //; s/\], 0x.*//' | cut -c1-120)
  gitpaths=$(grep -E "^$p " strace2.out | grep -o 'cli/\.git/[^"]*"[^)]*' | grep -v ENOENT | sed 's/".*//' | sort | uniq -c | tr '\n' ' ' | tr -s ' ')
  absent=$(grep -E "^$p " strace2.out | grep 'cli/\.git' | grep ENOENT | grep -o 'cli/\.git/[^"]*' | sort -u | tr '\n' ' ')
  echo "$p [$args] read: $gitpaths | absent: $absent"
done
echo === which test binaries exec git with cwd in repo: execve of .test binaries
grep -E 'execve\(".*\.test"' strace2.out | grep -o '"[^"]*\.test"' | sort | uniq -c | wc -l
