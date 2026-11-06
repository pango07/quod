// Diagnostic only: glibc loader notifications, never a replay certificate.
// Build on the Linux gate, not the developer's machine. The callbacks return
// the original symbol address and leave loader flags unchanged. This observes
// bindings, not calls, raw memory access, static calls, or failed lookups.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <link.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int audit_fd = -1;
struct line { char bytes[32768]; size_t used; };

static void byte(struct line *line, char c) {
  if (line->used == sizeof(line->bytes)) _exit(125);
  line->bytes[line->used++] = c;
}

static void field(struct line *line, const char *value) {
  static const char hex[] = "0123456789abcdef";
  byte(line, '\t');
  for (const unsigned char *p = (const unsigned char *)value; *p; ++p) {
    if (*p < 32 || *p >= 127 || *p == '%') {
      byte(line, '%'); byte(line, hex[*p >> 4]); byte(line, hex[*p & 15]);
    } else byte(line, (char)*p);
  }
}

static void number(struct line *line, unsigned long value) {
  char digits[32]; size_t n = 0;
  do { digits[n++] = (char)('0' + value % 10); value /= 10; } while (value);
  byte(line, '\t');
  while (n) byte(line, digits[--n]);
}

static void record(const char *kind, unsigned long flags,
                   const char *first, const char *second, const char *third) {
  if (audit_fd < 0) return;
  struct line line = {.used = 0};
  for (const char *p = kind; *p; ++p) byte(&line, *p);
  number(&line, (unsigned long)getpid()); number(&line, flags);
  field(&line, first); field(&line, second); field(&line, third); byte(&line, '\n');
  ssize_t n;
  do { n = write(audit_fd, line.bytes, line.used); } while (n < 0 && errno == EINTR);
  // A missing/partial diagnostic must fail its disposable run, not look clean.
  if (n < 0 || (size_t)n != line.used) _exit(125);
}

unsigned int la_version(unsigned int version) {
  const char *dir = getenv("QUOD_NATIVE_AUDIT_DIR");
  if (!dir || dir[0] != '/' || version < LAV_CURRENT) _exit(125);
  char path[4096];
  int n = snprintf(path, sizeof(path), "%s/loader-%lu.tsv", dir, (unsigned long)getpid());
  if (n < 0 || (size_t)n >= sizeof(path)) _exit(125);
  audit_fd = open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
  if (audit_fd < 0) _exit(125);
  record("version", LAV_CURRENT, "quod-native-symbol-audit-v1", "", "");
  return LAV_CURRENT;
}

unsigned int la_objopen(struct link_map *map, Lmid_t lmid, uintptr_t *cookie) {
  *cookie = (uintptr_t)map;
  record("object", (unsigned long)lmid, map->l_name, "", "");
  return LA_FLG_BINDTO | LA_FLG_BINDFROM;
}

uintptr_t la_symbind64(Elf64_Sym *sym, unsigned int ndx, uintptr_t *refcook,
                      uintptr_t *defcook, unsigned int *flags, const char *name) {
  (void)ndx;
  const struct link_map *from = (const struct link_map *)*refcook;
  const struct link_map *to = (const struct link_map *)*defcook;
  // Include all successful dynamic lookups and every binding from an addon.
  // Recording dlsym's provider matters: a preload may interpose a symbol.
  if ((*flags & LA_SYMB_DLSYM) || strstr(from->l_name, ".node"))
    record("bind", *flags, from->l_name, to->l_name, name);
  return sym->st_value;
}
