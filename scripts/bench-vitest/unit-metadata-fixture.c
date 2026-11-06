#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

static int emit(const char *label, long long value) {
  char line[128];
  int n = snprintf(line, sizeof(line), "%s=%lld\n", label, value);
  return n > 0 && n < (int)sizeof(line) && write(1, line, (size_t)n) == n ? 0 : 4;
}

/* Real syscall consumers, not a synthetic strace log. The audit changes
 * only metadata or a symlink's spelling between otherwise identical runs. */
int main(int argc, char **argv) {
  if (argc != 3) return 2;
  if (!strcmp(argv[1], "copy") || !strcmp(argv[1], "count")) {
    int input = open(argv[2], O_RDONLY);
    if (input < 0) return 3;
    int output = !strcmp(argv[1], "copy") ? open("out/o.txt", O_WRONLY | O_CREAT | O_TRUNC, 0644) : -1;
    if (!strcmp(argv[1], "copy") && output < 0) return 3;
    char bytes[4096];
    long long total = 0;
    ssize_t n;
    while ((n = read(input, bytes, sizeof(bytes))) > 0) {
      total += n;
      if (output >= 0 && write(output, bytes, (size_t)n) != n) return 4;
    }
    close(input);
    if (output >= 0) close(output);
    if (n < 0) return 3;
    return output >= 0 ? (write(1, "built\n", 6) == 6 ? 0 : 4) : emit("COUNT", total);
  }
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
    return emit("STAMP", (long long)sx.stx_mtime.tv_sec);
  } else if (!strcmp(argv[1], "readlink")) {
    char target[4096];
    ssize_t n = readlink(argv[2], target, sizeof(target));
    if (n < 0 || (size_t)n == sizeof(target)) return 3;
    return write(1, "TARGET=", 7) == 7 && write(1, target, (size_t)n) == n && write(1, "\n", 1) == 1 ? 0 : 4;
  }
  if (rc) return 3;
  return !strcmp(argv[1], "mode") ? emit("MODE", st.st_mode & 0777) : emit("STAMP", (long long)st.st_mtim.tv_sec);
}
