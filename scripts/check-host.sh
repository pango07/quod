#!/usr/bin/env bash
# Reports every row of the handoff's Environment table. Exit 1 if a 0a
# requirement fails; 0b rows are reported but only warn.
set -u
fail=0
row() { # level name ok detail
  local mark
  if [ "$3" = 1 ]; then mark=ok; elif [ "$1" = 0b ]; then mark=WARN; else mark=FAIL; fail=1; fi
  printf '%-5s %-4s %-44s %s\n' "$mark" "$1" "$2" "$4"
}
has() { command -v "$1" >/dev/null 2>&1; }

os=$(uname -s); arch=$(uname -m); kr=$(uname -r)
row 0a "Linux" "$([ "$os" = Linux ] && echo 1 || echo 0)" "$os $arch"
if [ "$arch" != x86_64 ]; then printf 'note       arch is %s, not x86_64 (see Open questions)\n' "$arch"; fi
kmaj=${kr%%.*}; krest=${kr#*.}; kmin=${krest%%.*}
row 0a "kernel >= 5.11" "$([ "$kmaj" -gt 5 ] || { [ "$kmaj" -eq 5 ] && [ "$kmin" -ge 11 ]; } && echo 1 || echo 0)" "$kr"
row 0a "/dev/fuse" "$([ -c /dev/fuse ] && echo 1 || echo 0)" ""
row 0a "fuse3 (fusermount3)" "$(has fusermount3 && echo 1 || echo 0)" "$(fusermount3 -V 2>/dev/null)"
row 0a "user_allow_other in /etc/fuse.conf" "$(grep -qs '^user_allow_other' /etc/fuse.conf || [ "$(id -u)" = 0 ] && echo 1 || echo 0)" "$([ "$(id -u)" = 0 ] && echo '(root: not needed)')"
row 0a "bubblewrap" "$(has bwrap && echo 1 || echo 0)" "$(bwrap --version 2>/dev/null)"
row 0a "unprivileged user namespaces" "$(unshare -Ur true 2>/dev/null && echo 1 || echo 0)" ""
row 0a "rust (cargo)" "$(has cargo && echo 1 || echo 0)" "$(cargo --version 2>/dev/null)"
want_go=$(sed -n '/^\[go\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../versions.toml")
have_go=$(go env GOVERSION 2>/dev/null)
row 0a "go $want_go" "$([ "$have_go" = "go$want_go" ] && echo 1 || echo 0)" "$have_go"
row 0a "sqlite3" "$(has sqlite3 && echo 1 || echo 0)" "$(sqlite3 --version 2>/dev/null | cut -d' ' -f1)"
row 0b "/dev/kvm writable" "$([ -w /dev/kvm ] && echo 1 || echo 0)" ""
row 0b "firecracker" "$(has firecracker && echo 1 || echo 0)" "$(firecracker --version 2>/dev/null | head -1)"
row 0b "virtiofsd" "$(has virtiofsd && echo 1 || echo 0)" "$(virtiofsd --version 2>/dev/null | head -1)"
row 0b "guest kernel and rootfs" "$([ -f "$(dirname "$0")/../guest/rootfs.ext4" ] && echo 1 || echo 0)" "scripts/build-guest.sh (T13)"
exit $fail
