#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

/* Real syscall consumers, not a synthetic strace log. The audit changes
 * only metadata or a symlink's spelling between otherwise identical runs. */
int main(int argc, char **argv) {
  if (argc != 3) return 2;
  struct stat st;
  int rc = -1;
  if (!strcmp(argv[1], "stat") || !strcmp(argv[1], "mode")) {
    rc = stat(argv[2], &st);
  } else if (!strcmp(argv[1], "lstat")) {
    rc = lstat(argv[2], &st);
  } else if (!strcmp(argv[1], "fstat") || !strcmp(argv[1], "empty-path")) {
    int fd = open(argv[2], O_RDONLY);
    if (fd < 0) return 3;
    rc = !strcmp(argv[1], "fstat")
      ? syscall(SYS_fstat, fd, &st)
      : fstatat(fd, "", &st, AT_EMPTY_PATH);
    close(fd);
  } else if (!strcmp(argv[1], "statx")) {
    struct statx sx;
    if (statx(AT_FDCWD, argv[2], 0, STATX_MTIME, &sx)) return 3;
    printf("STAMP=%lld\n", (long long)sx.stx_mtime.tv_sec);
    return 0;
  } else if (!strcmp(argv[1], "readlink")) {
    char target[4096];
    ssize_t n = readlink(argv[2], target, sizeof(target));
    if (n < 0 || (size_t)n == sizeof(target)) return 3;
    printf("TARGET=%.*s\n", (int)n, target);
    return 0;
  }
  if (rc) return 3;
  if (!strcmp(argv[1], "mode")) printf("MODE=%o\n", st.st_mode & 0777);
  else printf("STAMP=%lld\n", (long long)st.st_mtim.tv_sec);
  return 0;
}
