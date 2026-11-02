// pmwrap: a go -toolexec and -exec wrapper that records every process the
// Go command runs (compile, link, asm, vet, cgo, and each package's test
// binary) with its wall and CPU time and a content key of exactly what it
// reads, so a history of commits can be replayed offline to ask: how many
// of these processes had an unchanged read set, and how much time was in
// them? Measurement only; it never skips anything.
//
// Key per kind (sha256 of a canonical text):
//   compile: argv (with $WORK normalized) + content hash of every existing
//            file named on argv + of every archive named in -importcfg.
//   link:    argv + content hash of every file named on argv + of every
//            packagefile in -importcfg (the whole dependency closure).
//   asm/cgo: argv + content hash of every existing file named on argv.
//   vet:     argv + content hash of vet.cfg + of every existing file named
//            in it (GoFiles, NonGoFiles, PackageFile, PackageVetx values).
//   test:    the binary's content hash + argv + cwd + the Go testing
//            package's own access log (-test.testlogfile): getenv values,
//            and for every open/stat path the file's content hash (or type
//            and size for directories, or "absent").
// Each record also carries argv_key (argv alone) so a key change can be
// attributed to argv or to file contents.
package main

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"syscall"
	"time"
)

var workRe = regexp.MustCompile(`go-build\d+`)

// Paths under HOME before a test unit ran (test units only).
var homeBefore map[string]bool

func norm(s string) string { return workRe.ReplaceAllString(s, "go-build") }

func hashFile(p string) (string, int64, error) {
	f, err := os.Open(p)
	if err != nil {
		return "", 0, err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return "", 0, err
	}
	if st.IsDir() {
		return fmt.Sprintf("dir"), 0, nil
	}
	h := sha256.New()
	n, err := io.Copy(h, f)
	if err != nil {
		return "", 0, err
	}
	return hex.EncodeToString(h.Sum(nil))[:16], n, nil
}

type rec struct {
	Kind      string            `json:"kind"`
	Tool      string            `json:"tool"`
	Pkg       string            `json:"pkg,omitempty"`
	Out       string            `json:"out,omitempty"`
	Cwd       string            `json:"cwd"`
	Argv      []string          `json:"argv"`
	StartMs   int64             `json:"start_ms"`
	WallMs    int64             `json:"wall_ms"`
	UserMs    int64             `json:"user_ms"`
	SysMs     int64             `json:"sys_ms"`
	Exit      int               `json:"exit"`
	Key       string            `json:"key"`
	ArgvKey   string            `json:"argv_key"`
	// Content-only key: argv without any scratch or cache path ($WORK,
	// GOCACHE) and the sorted multiset of content hashes without paths. What
	// a tool-aware memo (Go's own cache) keys on; optimistic for a generic one.
	CKey      string            `json:"ckey"`
	NOwnWrite int               `json:"n_own_write,omitempty"`
	FilesKey  string            `json:"files_key"`
	NFiles    int               `json:"n_files"`
	NBytes    int64             `json:"n_bytes"`
	Files     map[string]string `json:"files,omitempty"`
	Getenv    map[string]string `json:"getenv,omitempty"`
	Orphans   int               `json:"orphans,omitempty"`
	ExecLike  []string          `json:"exec_like,omitempty"`
	Testlog   bool              `json:"testlog,omitempty"`
	NScratch  int               `json:"n_scratch,omitempty"`
	HashMs    int64             `json:"hash_ms"`
	LastLine  string            `json:"last_line,omitempty"`
}

func classify(tool string) string {
	b := filepath.Base(tool)
	if strings.HasSuffix(b, ".test") {
		return "test"
	}
	switch b {
	case "compile", "link", "asm", "vet", "cgo", "pack", "cover", "buildid", "preprofile":
		return b
	}
	return "other:" + b
}

func exists(p string) bool {
	_, err := os.Lstat(p)
	return err == nil
}

func abs(cwd, p string) string {
	if filepath.IsAbs(p) {
		return p
	}
	return filepath.Join(cwd, p)
}

// files named on argv that exist (sources, cfg files, archives).
func argvFiles(cwd string, args []string) []string {
	var out []string
	for i, a := range args {
		if a == "" || strings.HasPrefix(a, "-") && !strings.Contains(a, "=") {
			continue
		}
		cand := a
		if i := strings.IndexByte(a, '='); i >= 0 && strings.HasPrefix(a, "-") {
			cand = a[i+1:]
		}
		_ = i
		p := abs(cwd, cand)
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			out = append(out, p)
		}
	}
	return out
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

func vetcfgFiles(path string) []string {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var cfg map[string]any
	if json.Unmarshal(b, &cfg) != nil {
		return nil
	}
	var out []string
	var walk func(v any)
	walk = func(v any) {
		switch x := v.(type) {
		case string:
			if filepath.IsAbs(x) {
				if st, err := os.Stat(x); err == nil && !st.IsDir() {
					out = append(out, x)
				}
			}
		case []any:
			for _, e := range x {
				walk(e)
			}
		case map[string]any:
			for _, e := range x {
				walk(e)
			}
		}
	}
	walk(cfg)
	return out
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

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "pmwrap: usage: pmwrap TOOL ARGS...")
		os.Exit(2)
	}
	out := os.Getenv("PMWRAP_OUT")
	tool := os.Args[1]
	args := os.Args[2:]
	cwd, _ := os.Getwd()
	kind := classify(tool)
	r := rec{Kind: kind, Tool: tool, Cwd: cwd}

	testlog := ""
	if kind == "test" {
		for _, a := range args {
			if strings.HasPrefix(a, "-test.testlogfile=") {
				testlog = strings.TrimPrefix(a, "-test.testlogfile=")
			}
		}
		if testlog == "" {
			f, err := os.CreateTemp(os.Getenv("PMWRAP_TMP"), "testlog-")
			if err == nil {
				testlog = f.Name()
				f.Close()
				args = append(append([]string{}, args...), "-test.testlogfile="+testlog)
			}
		}
		// Become a subreaper so grandchildren orphaned by the test binary are
		// reaped here and counted (a lower bound on "spawned subprocesses").
		syscall.RawSyscall6(syscall.SYS_PRCTL, 36 /* PR_SET_CHILD_SUBREAPER */, 1, 0, 0, 0, 0)
		if home := os.Getenv("HOME"); home != "" {
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
	}

	// Version probes (go's tool id) are recorded as kind "version".
	for _, a := range args {
		if a == "-V=full" || a == "-V" {
			kind = "version"
			r.Kind = kind
		}
	}

	start := time.Now()
	cmd := exec.Command(tool, args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	var lastLine string
	if kind == "test" {
		// go test reads the binary's stdout; tee the last line for PASS/FAIL.
		pr, pw := io.Pipe()
		cmd.Stdout = pw
		done := make(chan struct{})
		go func() {
			sc := bufio.NewScanner(pr)
			sc.Buffer(make([]byte, 1<<20), 1<<26)
			for sc.Scan() {
				ln := sc.Text()
				if strings.TrimSpace(ln) != "" {
					lastLine = ln
				}
				fmt.Fprintln(os.Stdout, ln)
			}
			close(done)
		}()
		err := cmd.Run()
		pw.Close()
		<-done
		r.Exit = exitCode(err)
	} else {
		err := cmd.Run()
		r.Exit = exitCode(err)
	}
	r.WallMs = time.Since(start).Milliseconds()
	r.StartMs = start.UnixMilli()
	if cmd.ProcessState != nil {
		r.UserMs = cmd.ProcessState.UserTime().Milliseconds()
		r.SysMs = cmd.ProcessState.SystemTime().Milliseconds()
	}
	if kind == "test" {
		// Reap orphans the test binary left behind (a daemon it spawned and
		// did not wait for).
		for {
			var ws syscall.WaitStatus
			pid, err := syscall.Wait4(-1, &ws, syscall.WNOHANG, nil)
			if err != nil || pid <= 0 {
				break
			}
			r.Orphans++
		}
	}
	r.LastLine = lastLine

	// The read set.
	th := time.Now()
	nargs := make([]string, 0, len(args))
	for _, a := range args {
		// The injected test log path is random per run and not an input.
		if strings.HasPrefix(a, "-test.testlogfile=") {
			continue
		}
		nargs = append(nargs, norm(a))
	}
	// The toolexec'd tool path and the test binary path carry $WORK too.
	r.Argv = append([]string{norm(tool)}, nargs...)
	r.Out = norm(flagVal(args, "-o"))
	r.Pkg = flagVal(args, "-p")
	files := map[string]string{}
	var nbytes int64
	add := func(p string) {
		if _, ok := files[p]; ok {
			return
		}
		h, n, err := hashFile(p)
		if err != nil {
			files[p] = "absent"
			return
		}
		files[p] = h
		nbytes += n
	}
	switch kind {
	case "compile", "asm", "cgo", "pack", "cover":
		for _, p := range argvFiles(cwd, args) {
			add(p)
		}
		if ic := flagVal(args, "-importcfg"); ic != "" {
			for _, p := range importcfgFiles(abs(cwd, ic)) {
				add(p)
			}
		}
		if ec := flagVal(args, "-embedcfg"); ec != "" {
			// embedcfg names the embedded files: hash them too.
			b, _ := os.ReadFile(abs(cwd, ec))
			var cfg struct {
				Files map[string]string `json:"Files"`
			}
			if json.Unmarshal(b, &cfg) == nil {
				for _, p := range cfg.Files {
					add(abs(cwd, p))
				}
			}
		}
	case "link":
		for _, p := range argvFiles(cwd, args) {
			add(p)
		}
		if ic := flagVal(args, "-importcfg"); ic != "" {
			for _, p := range importcfgFiles(abs(cwd, ic)) {
				add(p)
			}
		}
	case "vet":
		for _, p := range argvFiles(cwd, args) {
			add(p)
			if strings.HasSuffix(p, "vet.cfg") {
				for _, q := range vetcfgFiles(p) {
					add(q)
				}
			}
		}
	case "test":
		add(tool)
		r.Getenv = map[string]string{}
		scratch := os.Getenv("PMWRAP_TMP")
		// What existed under HOME before this unit ran: a path under HOME the
		// testlog names that was not there is this unit's own write (a
		// config it wrote and read back), not an input. One that was there
		// was written by an earlier unit of the same step: an input, and a
		// cross-unit dependency.
		home := os.Getenv("HOME")
		if home == "" {
			homeBefore = nil
		}
		if testlog != "" {
			if f, err := os.Open(testlog); err == nil {
				r.Testlog = true
				pwd := cwd
				sc := bufio.NewScanner(f)
				sc.Buffer(make([]byte, 1<<20), 1<<26)
				for sc.Scan() {
					ln := sc.Text()
					if strings.HasPrefix(ln, "#") || ln == "" {
						continue
					}
					sp := strings.IndexByte(ln, ' ')
					if sp < 0 {
						continue
					}
					op, arg := ln[:sp], ln[sp+1:]
					switch op {
					case "getenv":
						r.Getenv[arg] = os.Getenv(arg)
					case "chdir":
						pwd = abs(pwd, arg)
					case "open", "stat":
						p := abs(pwd, arg)
						if isExecLike(p) {
							r.ExecLike = append(r.ExecLike, p)
						}
						// The process's own scratch (t.TempDir() under /tmp,
						// random names): written by this process before it
						// is read, so not an input. Counted, not keyed.
						if strings.HasPrefix(p, "/tmp/") || (scratch != "" && strings.HasPrefix(p, scratch)) || strings.HasPrefix(p, "/dev/") {
							r.NScratch++
							continue
						}
						if home != "" && homeBefore != nil && strings.HasPrefix(p, home+"/") && !homeBefore[p] {
							r.NOwnWrite++
							continue
						}
						if st, err := os.Lstat(p); err != nil {
							files[p] = "absent"
						} else if st.IsDir() {
							files[p] = "dir"
						} else if st.Mode().IsRegular() {
							add(p)
						} else {
							files[p] = fmt.Sprintf("%v", st.Mode().Type())
						}
					}
				}
				f.Close()
				if os.Getenv("PMWRAP_KEEP_TESTLOG") == "" && !strings.Contains(testlog, "go-build") {
					os.Remove(testlog)
				}
			}
		}
	}
	r.HashMs = time.Since(th).Milliseconds()
	r.NFiles = len(files)
	r.NBytes = nbytes

	// Keys.
	ah := sha256.New()
	fmt.Fprintf(ah, "%s\n%s\n", kind, strings.Join(r.Argv, "\x00"))
	if kind == "test" {
		fmt.Fprintf(ah, "cwd %s\n", cwd)
	}
	r.ArgvKey = hex.EncodeToString(ah.Sum(nil))[:16]
	keys := make([]string, 0, len(files))
	for p := range files {
		keys = append(keys, p)
	}
	sort.Strings(keys)
	fh := sha256.New()
	for _, p := range keys {
		fmt.Fprintf(fh, "%s=%s\n", norm(p), files[p])
	}
	if kind == "test" {
		ek := make([]string, 0, len(r.Getenv))
		for k := range r.Getenv {
			ek = append(ek, k)
		}
		sort.Strings(ek)
		for _, k := range ek {
			fmt.Fprintf(fh, "env %s=%s\n", k, r.Getenv[k])
		}
	}
	r.FilesKey = hex.EncodeToString(fh.Sum(nil))[:16]
	// Content-only key.
	gocache := os.Getenv("GOCACHE")
	ch := sha256.New()
	fmt.Fprintf(ch, "%s\n", kind)
	for _, a := range r.Argv {
		if strings.Contains(a, "go-build") || (gocache != "" && strings.Contains(a, gocache)) {
			continue
		}
		fmt.Fprintf(ch, "%s\x00", a)
	}
	hs := make([]string, 0, len(files))
	for _, h := range files {
		hs = append(hs, h)
	}
	sort.Strings(hs)
	fmt.Fprintf(ch, "\n%s\n", strings.Join(hs, ","))
	if kind == "test" {
		fmt.Fprintf(ch, "cwd %s\n", cwd)
		ek := make([]string, 0, len(r.Getenv))
		for k := range r.Getenv {
			ek = append(ek, k)
		}
		sort.Strings(ek)
		for _, k := range ek {
			fmt.Fprintf(ch, "env %s=%s\n", k, r.Getenv[k])
		}
	}
	r.CKey = hex.EncodeToString(ch.Sum(nil))[:16]
	kh := sha256.New()
	fmt.Fprintf(kh, "%s %s\n", r.ArgvKey, r.FilesKey)
	r.Key = hex.EncodeToString(kh.Sum(nil))[:16]
	if kind != "test" {
		files = nil
	} else {
		nf := map[string]string{}
		for p, h := range files {
			nf[norm(p)] = h
		}
		files = nf
	}
	r.Files = files
	if kind == "version" || kind == "other:go" {
		r.Files = nil
	}

	if out != "" {
		b, _ := json.Marshal(r)
		if f, err := os.OpenFile(out, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644); err == nil {
			syscall.Flock(int(f.Fd()), syscall.LOCK_EX)
			f.Write(append(b, '\n'))
			syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
			f.Close()
		}
	}
	os.Exit(r.Exit)
}

func isExecLike(p string) bool {
	for _, pre := range []string{"/usr/bin/", "/bin/", "/usr/local/go/bin/", "/usr/local/bin/", "/usr/sbin/", "/sbin/"} {
		if strings.HasPrefix(p, pre) {
			return true
		}
	}
	return false
}

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
