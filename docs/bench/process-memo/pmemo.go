// pmemo: a minimal process-level memo for Go's link actions and per-package
// test binaries, as a -toolexec / -exec shim. The prototype of
// docs/process-memo.md: everything else (compile, asm, vet) passes through.
//
// Store (PMEMO_STORE):
//   blobs/<sha256>                 content-addressed outputs and stdout
//   links/<key>.json               one record per link unit: the key is the
//                                  whole read set (argv + every input's content),
//                                  known before the link runs
//   tests/<lookup>/<manifest>.json manifests of a test unit (lookup = argv +
//                                  cwd + the binary's content); the read set is
//                                  only known after the run (the testing
//                                  package's log), so a lookup matches each
//                                  candidate manifest against the tree first,
//                                  exactly as Quod matches step manifests
// A record carries the result (exit, stdout blob, output blob) and a proof
// streak: how many consecutive executions of that unit with that read set
// produced that result. PMEMO_MODE=replay replays a unit whose streak is at
// least PMEMO_PROOFS (default 2) and whose exit was 0; observe records only.
// Every process appends one JSON line to PMEMO_LOG.
package main

import (
	"bufio"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"runtime"
	"strings"
	"sync/atomic"
	"syscall"
	"time"
	"unsafe"
)

var workRe = regexp.MustCompile(`go-build\d+`)

func norm(s string) string { return workRe.ReplaceAllString(s, "go-build") }

var store = os.Getenv("PMEMO_STORE")
var mode = os.Getenv("PMEMO_MODE")
var proofs = envInt("PMEMO_PROOFS", 2)
var cwd, _ = os.Getwd()
var homeBefore map[string]bool

func envInt(k string, d int) int {
	if v, err := strconv.Atoi(os.Getenv(k)); err == nil {
		return v
	}
	return d
}

// hashFileCached hashes a file once per inode identity (device, inode, size,
// mtime in ns) and remembers the hash in the store. Sound for files that are
// written once and never modified in place, which Go's cache entries and
// its $WORK outputs are; the design's answer is to serve the cache through
// the observer so hashes are known at write time (docs/process-memo.md 2.2).
// This stands in for that in the prototype.
func hashFileCached(p string) (string, error) {
	st, err := os.Stat(p)
	if err != nil || store == "" || !st.Mode().IsRegular() {
		return hashFile(p)
	}
	sys, ok := st.Sys().(*syscall.Stat_t)
	if !ok {
		return hashFile(p)
	}
	id := fmt.Sprintf("%d-%d-%d-%d", sys.Dev, sys.Ino, st.Size(), st.ModTime().UnixNano())
	cp := filepath.Join(store, "hc", id[:2], id)
	if b, err := os.ReadFile(cp); err == nil && len(b) == 64 {
		return string(b), nil
	}
	h, err := hashFile(p)
	if err != nil {
		return "", err
	}
	os.MkdirAll(filepath.Dir(cp), 0777)
	tmp := cp + ".tmp." + strconv.Itoa(os.Getpid())
	if os.WriteFile(tmp, []byte(h), 0666) == nil {
		os.Rename(tmp, cp)
	}
	return h, nil
}

func hashFile(p string) (string, error) {
	f, err := os.Open(p)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

func hashBytes(b []byte) string {
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}

func abs(p string) string {
	if filepath.IsAbs(p) {
		return p
	}
	return filepath.Join(cwd, p)
}

func flagVal(args []string, name string) string {
	for i, a := range args {
		if a == name && i+1 < len(args) {
			return args[i+1]
		}
		if strings.HasPrefix(a, name+"=") {
			return a[len(name)+1:]
		}
	}
	return ""
}

func importcfgFiles(path string) []string {
	f, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer f.Close()
	var out []string
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 1<<20), 1<<24)
	for sc.Scan() {
		ln := sc.Text()
		if strings.HasPrefix(ln, "packagefile ") || strings.HasPrefix(ln, "packageshlib ") {
			if i := strings.IndexByte(ln, '='); i >= 0 {
				out = append(out, ln[i+1:])
			}
		}
	}
	return out
}

func argvFiles(args []string) []string {
	var out []string
	for _, a := range args {
		cand := a
		if strings.HasPrefix(a, "-") {
			i := strings.IndexByte(a, '=')
			if i < 0 {
				continue
			}
			cand = a[i+1:]
		}
		p := abs(cand)
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			out = append(out, p)
		}
	}
	return out
}

// ---- store ----

func blobPath(h string) string { return filepath.Join(store, "blobs", h) }

func putBlobFile(p string) (string, error) {
	h, err := hashFile(p)
	if err != nil {
		return "", err
	}
	dst := blobPath(h)
	if _, err := os.Stat(dst); err == nil {
		return h, nil
	}
	tmp := dst + ".tmp." + strconv.Itoa(os.Getpid())
	if err := copyFile(p, tmp, 0444); err != nil {
		return "", err
	}
	if err := os.Rename(tmp, dst); err != nil {
		os.Remove(tmp)
		if _, e := os.Stat(dst); e == nil {
			return h, nil
		}
		return "", err
	}
	return h, nil
}

func putBlobBytes(b []byte) (string, error) {
	h := hashBytes(b)
	dst := blobPath(h)
	if _, err := os.Stat(dst); err == nil {
		return h, nil
	}
	tmp := dst + ".tmp." + strconv.Itoa(os.Getpid())
	if err := os.WriteFile(tmp, b, 0444); err != nil {
		return "", err
	}
	if err := os.Rename(tmp, dst); err != nil {
		os.Remove(tmp)
		return "", err
	}
	return h, nil
}

func copyFile(src, dst string, mode os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

type result struct {
	Exit   int    `json:"exit"`
	Stdout string `json:"stdout"`         // blob
	Out    string `json:"out,omitempty"`  // blob of the -o output (links)
	Mode   uint32 `json:"mode,omitempty"` // of the -o output
}

type record struct {
	Kind     string            `json:"kind"`
	Argv     []string          `json:"argv"`
	Cwd      string            `json:"cwd"`
	Files    map[string]string `json:"files"`            // path -> content hash | absent | dir
	Getenv   map[string]string `json:"getenv,omitempty"` // tests
	Result   result            `json:"result"`
	Streak   int               `json:"streak"`
	Runs     int               `json:"runs"`
	Replays  int               `json:"replays"`
	Duration int64             `json:"duration_ms"`
}

func (r *result) fingerprint() string {
	return fmt.Sprintf("%d %s %s %o", r.Exit, r.Stdout, r.Out, r.Mode)
}

// Update a record under its lock: same read set and same result extends the
// streak; a different result restarts it at 1. Returns the stored record.
func commit(path string, fresh *record) *record {
	os.MkdirAll(filepath.Dir(path), 0777)
	lf, err := os.OpenFile(path+".lock", os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return fresh
	}
	defer lf.Close()
	syscall.Flock(int(lf.Fd()), syscall.LOCK_EX)
	defer syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
	var old record
	if b, err := os.ReadFile(path); err == nil && json.Unmarshal(b, &old) == nil {
		if old.Result.fingerprint() == fresh.Result.fingerprint() {
			fresh.Streak = old.Streak + 1
		} else {
			fresh.Streak = 1
		}
		fresh.Runs = old.Runs + 1
		fresh.Replays = old.Replays
	} else {
		fresh.Streak = 1
		fresh.Runs = 1
	}
	b, _ := json.Marshal(fresh)
	tmp := path + ".tmp." + strconv.Itoa(os.Getpid())
	os.WriteFile(tmp, b, 0666)
	os.Rename(tmp, path)
	return fresh
}

func countReplay(path string) {
	lf, err := os.OpenFile(path+".lock", os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return
	}
	defer lf.Close()
	syscall.Flock(int(lf.Fd()), syscall.LOCK_EX)
	defer syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
	var r record
	if b, err := os.ReadFile(path); err == nil && json.Unmarshal(b, &r) == nil {
		r.Replays++
		b, _ = json.Marshal(r)
		tmp := path + ".tmp." + strconv.Itoa(os.Getpid())
		os.WriteFile(tmp, b, 0666)
		os.Rename(tmp, path)
	}
}

// Does the tree still match a recorded read set?
func matches(files map[string]string) (bool, string) {
	for p, h := range files {
		st, err := os.Lstat(p)
		switch {
		case h == "absent":
			if err == nil {
				return false, p
			}
		case h == "dir":
			if err != nil || !st.IsDir() {
				return false, p
			}
		default:
			if err != nil || !st.Mode().IsRegular() {
				return false, p
			}
			cur, err := hashFile(p)
			if err != nil || cur != h {
				return false, p
			}
		}
	}
	return true, ""
}

func replayable(r *record) bool {
	return mode == "replay" && r.Streak >= proofs && r.Result.Exit == 0
}

// ---- logging ----

type logLine struct {
	Kind      string `json:"kind"`
	Unit      string `json:"unit"`
	Outcome   string `json:"outcome"` // replayed | ran | ran-first | ran-proving | ran-changed | passthrough | opaque
	Why       string `json:"why,omitempty"`
	WallMs    int64  `json:"wall_ms"`
	LookupMs  int64  `json:"lookup_ms"`
	MaterMs   int64  `json:"materialize_ms"`
	RecordMs  int64  `json:"record_ms"`
	Exit      int    `json:"exit"`
	SavedMs   int64  `json:"saved_ms,omitempty"` // the recorded duration of a replayed unit
	Streak    int    `json:"streak,omitempty"`
	Candidates int   `json:"candidates,omitempty"`
	Scratch    int   `json:"scratch,omitempty"` // test units: /tmp and /dev paths left out of the key
}

func logit(l logLine) {
	out := os.Getenv("PMEMO_LOG")
	if out == "" {
		return
	}
	b, _ := json.Marshal(l)
	if f, err := os.OpenFile(out, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0666); err == nil {
		syscall.Flock(int(f.Fd()), syscall.LOCK_EX)
		f.Write(append(b, '\n'))
		syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
		f.Close()
	}
}

// ---- running ----

func exitCode(err error) int {
	if err == nil {
		return 0
	}
	if ee, ok := err.(*exec.ExitError); ok {
		if ws, ok := ee.Sys().(syscall.WaitStatus); ok && ws.Signaled() {
			return 128 + int(ws.Signal())
		}
		return ee.ExitCode()
	}
	return 127
}

// Run the tool, teeing its stdout (which the parent reads) into a buffer.
func run(tool string, args []string, capture bool) (int, []byte, int64) {
	start := time.Now()
	cmd := exec.Command(tool, args...)
	cmd.Stdin = os.Stdin
	cmd.Stderr = os.Stderr
	var buf []byte
	if capture {
		pr, pw := io.Pipe()
		cmd.Stdout = pw
		done := make(chan struct{})
		go func() {
			var sb strings.Builder
			w := io.MultiWriter(os.Stdout, &sb)
			io.Copy(w, pr)
			buf = []byte(sb.String())
			close(done)
		}()
		err := cmd.Run()
		pw.Close()
		<-done
		return exitCode(err), buf, time.Since(start).Milliseconds()
	}
	cmd.Stdout = os.Stdout
	err := cmd.Run()
	return exitCode(err), nil, time.Since(start).Milliseconds()
}

// ---- exec observation: seccomp user-notify on execve/execveat ----
//
// A test unit runs under a seccomp filter that reports every execve in its
// process tree to the shim (the design's observation mechanism, 2.2). The
// shim re-execs itself in child mode, which installs the filter with
// SECCOMP_FILTER_FLAG_NEW_LISTENER, hands the notify fd back over a socket
// pair, and execs the test binary; the first notification is that exec.
// Every exec is answered with CONTINUE, so nothing changes for the unit; the
// count is what makes it opaque. Unprivileged (no_new_privs). x86_64 only
// (the filter checks the arch and allows everything else).

const (
	sysSeccomp                   = 317
	seccompSetModeFilter         = 1
	seccompFlagNewListener       = 1 << 3
	seccompIoctlNotifRecv        = 0xC0502100 // _IOWR('!', 0, struct seccomp_notif[80])
	seccompIoctlNotifSend        = 0xC0182101 // _IOWR('!', 1, struct seccomp_notif_resp[24])
	seccompUserNotifFlagContinue = 1
	seccompRetAllow              = 0x7fff0000
	seccompRetUserNotif          = 0x7fc00000
	auditArchX86_64              = 0xC000003E
	prSetNoNewPrivs              = 38
	sysExecve                    = 59
	sysExecveat                  = 322
)

type sockFilter struct {
	Code uint16
	Jt   uint8
	Jf   uint8
	K    uint32
}

type sockFprog struct {
	Len    uint16
	_      [6]byte
	Filter *sockFilter
}

// Child mode: install the filter, send the listener to the parent on fd 3,
// exec the tool. On any failure before the exec, exec anyway: the parent
// then receives no fd and treats the unit as unobserved (opaque).
func seccompChild(tool string, args []string) {
	runtime.LockOSThread() // the filter attaches to the thread that execs
	prog := []sockFilter{
		{0x20, 0, 0, 4},                   // ld arch
		{0x15, 0, 4, auditArchX86_64},     // not x86_64: allow
		{0x20, 0, 0, 0},                   // ld nr
		{0x15, 1, 0, sysExecve},           // execve: notify
		{0x15, 0, 1, sysExecveat},         // execveat: notify, else allow
		{0x06, 0, 0, seccompRetUserNotif}, // notify
		{0x06, 0, 0, seccompRetAllow},     // allow
	}
	fprog := sockFprog{Len: uint16(len(prog)), Filter: &prog[0]}
	if _, _, e := syscall.RawSyscall(syscall.SYS_PRCTL, prSetNoNewPrivs, 1, 0); e == 0 {
		fd, _, e := syscall.RawSyscall(sysSeccomp, seccompSetModeFilter, seccompFlagNewListener, uintptr(unsafe.Pointer(&fprog)))
		if e == 0 {
			rights := syscall.UnixRights(int(fd))
			syscall.Sendmsg(3, []byte{1}, rights, nil, 0)
			syscall.Close(int(fd))
		}
	}
	syscall.Close(3)
	syscall.Exec(tool, append([]string{tool}, args...), os.Environ())
	os.Exit(127)
}

// runTraced runs a test binary under the exec filter, teeing its stdout.
// Returns the exit code, stdout, wall, and the number of execs observed in
// the unit's subtree beyond the binary's own (-1: the filter could not be
// installed, so nothing was observed).
func runTraced(tool string, args []string) (int, []byte, int64, int) {
	self, err := os.Executable()
	if err != nil {
		code, out, wall := run(tool, args, true)
		return code, out, wall, -1
	}
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM|syscall.SOCK_CLOEXEC, 0)
	if err != nil {
		code, out, wall := run(tool, args, true)
		return code, out, wall, -1
	}
	start := time.Now()
	cmd := exec.Command(self, append([]string{"--seccomp-child", tool}, args...)...)
	cmd.Stdin = os.Stdin
	cmd.Stderr = os.Stderr
	cmd.ExtraFiles = []*os.File{os.NewFile(uintptr(fds[1]), "notify-sock")}
	pr, pw := io.Pipe()
	cmd.Stdout = pw
	done := make(chan struct{})
	var buf []byte
	go func() {
		var sb strings.Builder
		io.Copy(io.MultiWriter(os.Stdout, &sb), pr)
		buf = []byte(sb.String())
		close(done)
	}()
	if err := cmd.Start(); err != nil {
		pw.Close()
		<-done
		syscall.Close(fds[0])
		syscall.Close(fds[1])
		return 127, buf, time.Since(start).Milliseconds(), -1
	}
	cmd.ExtraFiles[0].Close()
	// Receive the listener.
	execs := -1
	data := make([]byte, 1)
	oob := make([]byte, 64)
	n, oobn, _, _, err := syscall.Recvmsg(fds[0], data, oob, 0)
	syscall.Close(fds[0])
	var counter int64
	if err == nil && n > 0 && oobn > 0 {
		if msgs, err := syscall.ParseSocketControlMessage(oob[:oobn]); err == nil && len(msgs) > 0 {
			if nfds, err := syscall.ParseUnixRights(&msgs[0]); err == nil && len(nfds) > 0 {
				execs = 0
				go notifyLoop(nfds[0], &counter)
			}
		}
	}
	err = cmd.Wait()
	pw.Close()
	<-done
	code := exitCode(err)
	if execs == 0 {
		// The first exec is the test binary itself.
		execs = int(atomic.LoadInt64(&counter)) - 1
		if execs < 0 {
			execs = 0
		}
	}
	return code, buf, time.Since(start).Milliseconds(), execs
}

// notifyLoop answers every exec notification with CONTINUE and counts it.
func notifyLoop(fd int, counter *int64) {
	req := make([]byte, 80)
	resp := make([]byte, 24)
	for {
		for i := range req {
			req[i] = 0
		}
		_, _, e := syscall.Syscall(syscall.SYS_IOCTL, uintptr(fd), seccompIoctlNotifRecv, uintptr(unsafe.Pointer(&req[0])))
		if e == syscall.EINTR {
			continue
		}
		if e != 0 {
			return
		}
		atomic.AddInt64(counter, 1)
		for i := range resp {
			resp[i] = 0
		}
		copy(resp[0:8], req[0:8]) // id
		binary.LittleEndian.PutUint32(resp[20:24], seccompUserNotifFlagContinue)
		syscall.Syscall(syscall.SYS_IOCTL, uintptr(fd), seccompIoctlNotifSend, uintptr(unsafe.Pointer(&resp[0])))
	}
}

// liveDescendants counts processes whose parent is this shim: after the test
// binary exited, those are its orphans still running (we are a subreaper).
func liveDescendants() int {
	me := strconv.Itoa(os.Getpid())
	ents, err := os.ReadDir("/proc")
	if err != nil {
		return 0
	}
	n := 0
	for _, e := range ents {
		if e.Name()[0] < '0' || e.Name()[0] > '9' || e.Name() == me {
			continue
		}
		b, err := os.ReadFile("/proc/" + e.Name() + "/stat")
		if err != nil {
			continue
		}
		s := string(b)
		i := strings.LastIndexByte(s, ')')
		if i < 0 {
			continue
		}
		f := strings.Fields(s[i+1:])
		if len(f) > 1 && f[1] == me {
			n++
		}
	}
	return n
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "pmemo: usage: pmemo TOOL ARGS...")
		os.Exit(2)
	}
	if os.Args[1] == "--seccomp-child" && len(os.Args) >= 3 {
		seccompChild(os.Args[2], os.Args[3:])
	}
	tool, args := os.Args[1], os.Args[2:]
	base := filepath.Base(tool)
	if store == "" {
		code, _, _ := run(tool, args, false)
		os.Exit(code)
	}
	os.MkdirAll(filepath.Join(store, "blobs"), 0777)
	for _, a := range args {
		if a == "-V=full" || a == "-V" {
			code, _, _ := run(tool, args, false)
			os.Exit(code)
		}
	}
	switch {
	case base == "link":
		os.Exit(doLink(tool, args))
	case strings.HasSuffix(base, ".test"):
		os.Exit(doTest(tool, args))
	default:
		code, _, wall := run(tool, args, false)
		logit(logLine{Kind: base, Unit: norm(flagVal(args, "-o")), Outcome: "passthrough", WallMs: wall, Exit: code})
		os.Exit(code)
	}
}

func doLink(tool string, args []string) int {
	t0 := time.Now()
	out := abs(flagVal(args, "-o"))
	files := map[string]string{}
	for _, p := range argvFiles(args) {
		if h, err := hashFileCached(p); err == nil {
			files[p] = h
		}
	}
	if ic := flagVal(args, "-importcfg"); ic != "" {
		for _, p := range importcfgFiles(abs(ic)) {
			if _, ok := files[p]; ok {
				continue
			}
			if h, err := hashFileCached(p); err == nil {
				files[p] = h
			} else {
				files[p] = "absent"
			}
		}
	}
	nargv := make([]string, 0, len(args)+1)
	nargv = append(nargv, norm(tool))
	for _, a := range args {
		nargv = append(nargv, norm(a))
	}
	// The key: argv (paths normalized) and every input's content, by path.
	kh := sha256.New()
	fmt.Fprintf(kh, "link\n%s\n", strings.Join(nargv, "\x00"))
	paths := make([]string, 0, len(files))
	for p := range files {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	for _, p := range paths {
		fmt.Fprintf(kh, "%s=%s\n", norm(p), files[p])
	}
	key := hex.EncodeToString(kh.Sum(nil))
	recPath := filepath.Join(store, "links", key[:2], key+".json")
	lookup := time.Since(t0).Milliseconds()
	unit := norm(out)

	var old record
	if b, err := os.ReadFile(recPath); err == nil && json.Unmarshal(b, &old) == nil && replayable(&old) {
		tm := time.Now()
		err := copyFile(blobPath(old.Result.Out), out, os.FileMode(old.Result.Mode))
		if err == nil {
			if so, err := os.ReadFile(blobPath(old.Result.Stdout)); err == nil {
				os.Stdout.Write(so)
			}
			countReplay(recPath)
			logit(logLine{Kind: "link", Unit: unit, Outcome: "replayed", LookupMs: lookup,
				MaterMs: time.Since(tm).Milliseconds(), WallMs: time.Since(t0).Milliseconds(),
				Exit: old.Result.Exit, SavedMs: old.Duration, Streak: old.Streak})
			return old.Result.Exit
		}
		// A missing blob: run.
	}
	code, stdout, wall := run(tool, args, true)
	tr := time.Now()
	res := result{Exit: code}
	res.Stdout, _ = putBlobBytes(stdout)
	if st, err := os.Stat(out); err == nil && code == 0 {
		res.Mode = uint32(st.Mode().Perm())
		res.Out, _ = putBlobFile(out)
	}
	rec := commit(recPath, &record{Kind: "link", Argv: nargv, Cwd: cwd, Files: files, Result: res, Duration: wall})
	outcome := "ran-proving"
	if rec.Runs == 1 {
		outcome = "ran-first"
	} else if rec.Streak == 1 {
		outcome = "ran-changed"
	}
	if mode != "replay" {
		outcome = "ran"
	}
	logit(logLine{Kind: "link", Unit: unit, Outcome: outcome, LookupMs: lookup, WallMs: wall,
		RecordMs: time.Since(tr).Milliseconds(), Exit: code, Streak: rec.Streak})
	return code
}

func doTest(tool string, args []string) int {
	t0 := time.Now()
	binHash, err := hashFile(tool)
	if err != nil {
		code, _, _ := run(tool, args, false)
		return code
	}
	testlog := ""
	nargv := []string{"test"}
	for _, a := range args {
		if strings.HasPrefix(a, "-test.testlogfile=") {
			testlog = strings.TrimPrefix(a, "-test.testlogfile=")
			continue
		}
		nargv = append(nargv, norm(a))
	}
	lh := sha256.New()
	fmt.Fprintf(lh, "test\n%s\n%s\ncwd %s\n", binHash, strings.Join(nargv, "\x00"), cwd)
	lookupKey := hex.EncodeToString(lh.Sum(nil))
	dir := filepath.Join(store, "tests", lookupKey[:2], lookupKey)
	unit := strings.TrimPrefix(cwd, os.Getenv("PMEMO_WS")+"/")

	// Candidates: every manifest of this lookup key; the first whose read set
	// the tree still matches and whose result is proven replays.
	var candidates int
	if ents, err := os.ReadDir(dir); err == nil {
		type cand struct {
			path string
			rec  record
			mod  time.Time
		}
		var cs []cand
		for _, e := range ents {
			if !strings.HasSuffix(e.Name(), ".json") {
				continue
			}
			p := filepath.Join(dir, e.Name())
			var r record
			if b, err := os.ReadFile(p); err == nil && json.Unmarshal(b, &r) == nil {
				info, _ := e.Info()
				cs = append(cs, cand{p, r, info.ModTime()})
			}
		}
		sort.Slice(cs, func(i, j int) bool { return cs[i].mod.After(cs[j].mod) })
		candidates = len(cs)
		for _, c := range cs {
			ok, _ := matches(c.rec.Files)
			if !ok {
				continue
			}
			envOk := true
			for k, v := range c.rec.Getenv {
				if os.Getenv(k) != v {
					envOk = false
					break
				}
			}
			if !envOk {
				continue
			}
			if replayable(&c.rec) {
				if so, err := os.ReadFile(blobPath(c.rec.Result.Stdout)); err == nil {
					os.Stdout.Write(so)
					countReplay(c.path)
					logit(logLine{Kind: "test", Unit: unit, Outcome: "replayed", LookupMs: time.Since(t0).Milliseconds(),
						WallMs: time.Since(t0).Milliseconds(), Exit: c.rec.Result.Exit, SavedMs: c.rec.Duration,
						Streak: c.rec.Streak, Candidates: candidates})
					return c.rec.Result.Exit
				}
			}
			break // the matching manifest is not yet proven: run and extend its streak
		}
	}
	lookup := time.Since(t0).Milliseconds()

	// Run, with the testing package's log injected, as a subreaper, under
	// the exec filter. Files created from here on are the unit's own.
	unitStart := time.Now()
	if testlog == "" {
		if f, err := os.CreateTemp(os.Getenv("PMEMO_TMP"), "testlog-"); err == nil {
			testlog = f.Name()
			f.Close()
			args = append(append([]string{}, args...), "-test.testlogfile="+testlog)
		}
	}
	syscall.RawSyscall6(syscall.SYS_PRCTL, 36, 1, 0, 0, 0, 0)
	home := os.Getenv("HOME")
	if home != "" {
		homeBefore = map[string]bool{}
		filepath.WalkDir(home, func(p string, d os.DirEntry, err error) error {
			if err == nil {
				homeBefore[p] = true
			}
			if len(homeBefore) > 20000 {
				return filepath.SkipAll
			}
			return nil
		})
	}
	code, stdout, wall, execs := runTraced(tool, args)
	live := liveDescendants()
	for {
		var ws syscall.WaitStatus
		pid, err := syscall.Wait4(-1, &ws, syscall.WNOHANG, nil)
		if err != nil || pid <= 0 {
			break
		}
	}
	tr := time.Now()
	// The binary is in the lookup key by content (its $WORK path is random
	// per run), so it is not an entry of the manifest.
	files := map[string]string{}
	getenv := map[string]string{}
	opaque := ""
	nScratch := 0
	switch {
	case execs < 0:
		opaque = "exec observation unavailable"
	case execs > 0:
		// The testing log does not see what a subprocess reads. Over-record: refuse.
		opaque = fmt.Sprintf("spawned %d subprocess(es)", execs)
	case live > 0:
		opaque = fmt.Sprintf("%d subprocess(es) still alive", live)
	}
	if f, err := os.Open(testlog); err == nil {
		pwd := cwd
		sc := bufio.NewScanner(f)
		sc.Buffer(make([]byte, 1<<20), 1<<26)
		for sc.Scan() {
			ln := sc.Text()
			sp := strings.IndexByte(ln, ' ')
			if strings.HasPrefix(ln, "#") || sp < 0 {
				continue
			}
			op, arg := ln[:sp], ln[sp+1:]
			switch op {
			case "getenv":
				getenv[arg] = os.Getenv(arg)
			case "chdir":
				if filepath.IsAbs(arg) {
					pwd = arg
				} else {
					pwd = filepath.Join(pwd, arg)
				}
			case "open", "stat":
				p := arg
				if !filepath.IsAbs(p) {
					p = filepath.Join(pwd, p)
				}
				if strings.HasPrefix(p, "/dev/") {
					nScratch++
					continue
				}
				if strings.HasPrefix(p, "/tmp/") || (os.Getenv("PMEMO_TMP") != "" && strings.HasPrefix(p, os.Getenv("PMEMO_TMP")+"/")) {
					// Scratch only if this unit created it (ctime after the
					// unit started) or it does not exist; a /tmp file that
					// predates the unit (a sibling's, an earlier run's) is
					// an input and is keyed like any other.
					st, err := os.Lstat(p)
					if err != nil {
						nScratch++
						continue
					}
					if sys, ok := st.Sys().(*syscall.Stat_t); ok && time.Unix(sys.Ctim.Sec, sys.Ctim.Nsec).After(unitStart) {
						nScratch++
						continue
					}
					if p == tool {
						continue // the binary: keyed by content in the lookup
					}
				}
				if home != "" && strings.HasPrefix(p, home+"/") && !homeBefore[p] {
					continue // written by this unit
				}
				if _, ok := files[p]; ok {
					continue
				}
				st, err := os.Lstat(p)
				switch {
				case err != nil:
					files[p] = "absent"
				case st.IsDir():
					files[p] = "dir"
				case st.Mode().IsRegular():
					if h, err := hashFileCached(p); err == nil {
						files[p] = h
					} else {
						opaque = "unreadable input " + p
					}
				default:
					opaque = "non-regular input " + p
				}
			}
		}
		f.Close()
		os.Remove(testlog)
	} else {
		opaque = "no test log"
	}
	if opaque != "" {
		logit(logLine{Kind: "test", Unit: unit, Outcome: "opaque", Why: opaque, LookupMs: lookup, WallMs: wall, Exit: code, Candidates: candidates, Scratch: nScratch})
		return code
	}
	res := result{Exit: code}
	res.Stdout, _ = putBlobBytes(stdout)
	mh := sha256.New()
	paths := make([]string, 0, len(files))
	for p := range files {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	for _, p := range paths {
		fmt.Fprintf(mh, "%s=%s\n", norm(p), files[p])
	}
	ek := make([]string, 0, len(getenv))
	for k := range getenv {
		ek = append(ek, k)
	}
	sort.Strings(ek)
	for _, k := range ek {
		fmt.Fprintf(mh, "env %s=%s\n", k, getenv[k])
	}
	manifest := hex.EncodeToString(mh.Sum(nil))
	rec := commit(filepath.Join(dir, manifest+".json"), &record{Kind: "test", Argv: nargv, Cwd: cwd, Files: files, Getenv: getenv, Result: res, Duration: wall})
	outcome := "ran-proving"
	if rec.Runs == 1 {
		outcome = "ran-first"
	} else if rec.Streak == 1 {
		outcome = "ran-changed"
	}
	if mode != "replay" {
		outcome = "ran"
	}
	logit(logLine{Kind: "test", Unit: unit, Outcome: outcome, LookupMs: lookup, WallMs: wall,
		RecordMs: time.Since(tr).Milliseconds(), Exit: code, Streak: rec.Streak, Candidates: candidates})
	return code
}
