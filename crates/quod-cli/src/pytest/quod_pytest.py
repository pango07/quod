#!/usr/bin/env python3
# quod pytest: test-file-level replay with shadow recording.
#
# Written by `quod pytest` into <dir>/plugin/ and run as
#     python quod_pytest.py <opts.json> <pytest args...>
# OPTS["mode"]:
#   critical  the developer's job: select against the most recent completed
#             ledger, run the affected test files plainly (no recording) in
#             one pytest session, replay the rest through pytest's own hooks
#   shadow    the recording job: same selection, the affected files run with
#             recording; files whose results match the critical path's are
#             recorded, certified (alone in a fresh process == in session) or
#             not, and the ledger is written
#   alone     one test file in a fresh process (certification)
#   plain     no ledger: every file runs (a guard's full re-run)
# Design and residuals: docs/pytest-shadow.md (pango07/quod).

import ast
import builtins
import difflib
import fnmatch
import hashlib
import json
import os
import re
import stat as _statmod
import subprocess
import sys
import threading
import time
import types

LEDGER_VERSION = 1
SETTLE_NS = 2_000_000_000
_ME = os.path.abspath(__file__)
with open(_ME, "rb") as _f:
    PLUGIN_HASH = hashlib.blake2b(_f.read(), digest_size=16).hexdigest()
del _f

OPTS = {}
REC = None  # the Recorder (shadow only)
WS = os.getcwd()  # the workspace root; pytest's rootdir once configured
START = {}
_real_stat = os.stat
_real_lstat = os.lstat
_real_listdir = os.listdir


def h(b):
    return hashlib.blake2b(b, digest_size=16).hexdigest()


def hs(s):
    return h(s.encode("utf-8", "surrogateescape"))


def log(msg):
    sys.stderr.write("quod: %s\n" % msg)
    sys.stderr.flush()


def rel(path):
    """A key's path: relative to the workspace when inside it."""
    p = os.path.normpath(path)
    if p == WS:
        return "."
    if p.startswith(WS + os.sep):
        return p[len(WS) + 1 :]
    return p


def absp(p):
    return p if os.path.isabs(p) else os.path.normpath(os.path.join(WS, p))


def norm_text(s):
    """For matching results only (never for replayed output): object
    addresses differ between faithful runs."""
    return re.sub(r"0x[0-9a-fA-F]{4,}", "0x?", s)


# ---------------------------------------------------------------------------
# Key states


def _norm_dist(name):
    return re.sub(r"[-_.]+", "-", name).lower()


def own_dists():
    """dist-info dir -> normalized name, for this project's own editable
    installs (direct_url.json: editable, a file URL inside the workspace).
    Their version metadata is not an input unless the code reads it."""
    import importlib.metadata as md

    out = {}
    for d in md.distributions():
        p = getattr(d, "_path", None)
        if p is None:
            continue
        try:
            du = json.loads(d.read_text("direct_url.json") or "null")
        except Exception:
            du = None
        if not isinstance(du, dict):
            continue
        url = du.get("url", "")
        if not url.startswith("file://"):
            continue
        from urllib.parse import unquote, urlparse

        path = os.path.normpath(unquote(urlparse(url).path))
        cwd = os.getcwd()
        if path == cwd or path.startswith(cwd + os.sep) or cwd.startswith(path + os.sep):
            name = os.path.basename(str(p)).split("-")[0]
            out[os.path.normpath(str(p))] = _norm_dist(name)
    return out


class States:
    """Current states of keys, memoized; file contents through a stat cache
    (an entry is kept only for files older than SETTLE_NS when hashed: git's
    racy rule)."""

    def __init__(self, stat_cache, own=None):
        self.sc = stat_cache or {}
        self.new_sc = {}
        self.memo = {}
        self.listings = {}
        self._own = own
        self._dists = None

    def content(self, path):
        try:
            st = _real_stat(path)
        except FileNotFoundError:
            return "absent"
        except NotADirectoryError:
            return "absent"
        except OSError as e:
            return "err:%s" % e.errno
        if _statmod.S_ISDIR(st.st_mode):
            return "dir"
        ident = [st.st_size, st.st_mtime_ns, st.st_ino, st.st_ctime_ns]
        c = self.sc.get(path)
        if c is not None and c[:4] == ident:
            self.new_sc[path] = c
            return c[4]
        try:
            with open(path, "rb") as f:
                hv = h(f.read())
        except OSError as e:
            return "err:%s" % e.errno
        if time.time_ns() - st.st_mtime_ns > SETTLE_NS:
            self.new_sc[path] = ident + [hv]
        return hv

    def listing(self, d):
        r = self.listings.get(d)
        if r is None:
            try:
                r = sorted(_real_listdir(d))
            except OSError:
                r = None
            self.listings[d] = r
        return r

    def own(self):
        if self._own is None:
            self._own = own_dists()
        return self._own

    def state(self, key):
        v = self.memo.get(key)
        if v is None:
            try:
                v = self._state(key)
            except Exception as e:  # an unreadable key is a changed key
                v = "error:%s" % type(e).__name__
            self.memo[key] = v
        return v

    def _state(self, key):
        kind, _, arg = key.partition(":")
        if kind in ("f", "m"):
            return self.content(absp(arg))
        if kind == "e":
            try:
                st = _real_stat(absp(arg))
            except OSError:
                return "absent"
            if _statmod.S_ISDIR(st.st_mode):
                return "dir"
            if _statmod.S_ISREG(st.st_mode):
                return "file:%d" % st.st_size
            return "other"
        if kind == "l":
            r = self.listing(absp(arg))
            return "absent" if r is None else hs("\0".join(r))
        if kind == "x":
            d, _, stem = arg.rpartition("/")
            r = self.listing(absp(d))
            if r is None:
                return "absent"
            return hs("\0".join(n for n in r if n == stem or n.startswith(stem + ".")))
        if kind == "v":
            v = os.environ.get(arg)
            return "unset" if v is None else hs(v)
        if kind == "venv":
            return env_all()
        if kind == "od":
            name, _, fname = arg.partition("/")
            for p, n in self.own().items():
                if n == name:
                    return self.content(os.path.join(p, fname))
            return "absent"
        if kind == "ini":
            return ini_state(absp(arg))
        if kind == "py":
            return py_state()
        if kind == "sp":
            return hs(json.dumps(START.get("sys_path", [])))
        if kind == "mp":
            return START.get("meta", "")
        if kind == "dists":
            return self.dists()[0]
        if kind == "d":
            return self.dists()[1].get(arg, "absent")
        return "unknown-kind"

    def dists(self):
        """(hash of the installed names, name -> content state), every
        distribution on sys.path but this project's own editable install;
        keyed by name and file content, not by dist-info dir names."""
        if self._dists is not None:
            return self._dists
        import importlib.metadata as md

        own = self.own()
        per = {}
        seen = set()
        owned = {}
        bases = set()
        for d in md.distributions():
            p = getattr(d, "_path", None)
            if p is None:
                continue
            p = os.path.normpath(str(p))
            if p in seen:
                continue
            seen.add(p)
            base = os.path.dirname(p)
            bases.add(base)
            dirname = os.path.basename(p)
            files = []
            try:
                with open(os.path.join(p, "RECORD"), encoding="utf-8") as f:
                    for line in f:
                        files.append(line.rsplit(",", 2)[0].strip().strip('"'))
            except OSError:
                files = []
            for fn in files:
                owned[os.path.normpath(os.path.join(base, fn))] = True
            if p in own:
                continue
            name = _norm_dist(dirname.split("-")[0])
            parts = []
            for fn in files:
                if "__pycache__" in fn or fn.endswith(".pyc"):
                    continue
                if fn.startswith(dirname + "/"):
                    leaf = fn[len(dirname) + 1 :]
                    if leaf in ("RECORD", "INSTALLER", "REQUESTED", "direct_url.json", "uv_cache.json"):
                        continue
                    fn_key = "<dist-info>/" + leaf
                else:
                    fn_key = fn
                parts.append(fn_key + "=" + self.content(os.path.normpath(os.path.join(base, fn))))
            if not files:
                parts.append("norecord:" + dirname)
            parts.sort()
            per[name] = hs("\n".join(parts))
        # Top-level files no distribution owns (.pth files run at start-up).
        for base in sorted(bases):
            for n in self.listing(base) or ():
                full = os.path.join(base, n)
                if (n.endswith(".pth") or n.endswith(".py")) and full not in owned:
                    per["file:" + n] = self.content(full)
        names = hs("\n".join(sorted(per)))
        self._dists = (names, per)
        return self._dists


_VOLATILE = None


def volatile(name):
    global _VOLATILE
    if _VOLATILE is None:
        _VOLATILE = set(OPTS.get("volatile", []))
    return name in _VOLATILE or name.startswith("QUOD_")


def env_all():
    return hs("\0".join("%s=%s" % kv for kv in sorted(os.environ.items()) if not volatile(kv[0])))


def py_state():
    import platform

    facts = [
        sys.version,
        os.path.realpath(sys.executable),
        sys.prefix,
        sys.base_prefix,
        sys.platform,
        platform.machine(),
        sys.implementation.cache_tag or "",
        repr(sys.flags),
    ]
    for k in sorted(os.environ):
        if k.startswith("PYTHON"):
            facts.append("%s=%s" % (k, os.environ[k]))
    return hs("\n".join(facts))


def ini_state(path):
    """pytest's own section of a shared config file: a version bump or a
    linter setting in pyproject.toml is not an input of any test."""
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError:
        return "absent"
    name = os.path.basename(path)
    try:
        if name == "pyproject.toml":
            import tomllib

            tool = tomllib.loads(data.decode("utf-8")).get("tool", {})
            return "toml:" + hs(json.dumps(tool.get("pytest"), sort_keys=True, default=str))
        if name in ("tox.ini", "setup.cfg"):
            import iniconfig

            cfg = iniconfig.IniConfig(path, data.decode("utf-8"))
            out = {}
            for sec in ("pytest", "tool:pytest"):
                if sec in cfg:
                    out[sec] = sorted(cfg[sec].items())
            return "ini:" + hs(json.dumps(out, sort_keys=True))
    except Exception:
        pass
    return h(data)


# ---------------------------------------------------------------------------
# Recording (shadow only)


class Window:
    __slots__ = ("kind", "ident", "reads", "imports", "failed", "exec", "unrec", "env")

    def __init__(self, kind, ident):
        self.kind = kind
        self.ident = ident
        self.reads = {}  # key -> None (ordered set)
        self.imports = set()
        self.failed = set()
        self.exec = set()
        self.unrec = None
        self.env = {}

    def taint(self, why):
        if self.unrec is None:
            self.unrec = why


_TRANSPARENT = {
    "pathlib",
    "pathlib._local",
    "pathlib._abc",
    "io",
    "_pyio",
    "codecs",
    "tokenize",
    "genericpath",
    "posixpath",
    "os",
    "shutil",
    "glob",
    "fnmatch",
    "tempfile",
    "linecache",
}


def _caller():
    f = sys._getframe(2)
    me = __name__
    while f is not None:
        g = f.f_globals.get("__name__", "") or ""
        if g != me and g not in _TRANSPARENT:
            return g
        f = f.f_back
    return ""


_LOCALHOST = ("127.0.0.1", "localhost", "::1", "0.0.0.0", "")


class Recorder:
    """Per-test-file attribution inside one pytest process: windows for the
    session, each test file (its collection and its tests) and each module's
    first load; every read, stat, listing, env read, import and executed
    function goes to the current window."""

    def __init__(self):
        self.on = False
        self.paused = 0
        self.S = Window("s", "")
        self.stack = [self.S]
        self.files = {}
        self.loads = {}
        self.written = set()
        self.env_written = set()
        self.loading = 0
        self.excl = ()
        self.own = {}
        self.mon = None
        self.thread_start = None

    @property
    def cur(self):
        return self.stack[-1]

    def file(self, relpath):
        w = self.files.get(relpath)
        if w is None:
            w = self.files[relpath] = Window("f", relpath)
        return w

    def push(self, w):
        self.stack.append(w)
        self._restart()

    def pop(self):
        if len(self.stack) > 1:
            self.stack.pop()
        self._restart()

    def _restart(self):
        if self.mon is not None:
            try:
                sys.monitoring.restart_events()
            except Exception:
                pass

    # -- install -----------------------------------------------------------

    def install(self):
        import sysconfig
        import tempfile

        self.own = own_dists()
        pre = set()
        paths = sysconfig.get_paths()
        for k in ("stdlib", "platstdlib", "purelib", "platlib"):
            if paths.get(k):
                pre.add(paths[k])
        for p in (sys.prefix, sys.base_prefix, sys.exec_prefix, sys.base_exec_prefix):
            pre.add(p)
        for p in sys.path:
            if p.endswith("site-packages") or p.endswith("dist-packages"):
                pre.add(p)
        tmp = tempfile.gettempdir()
        cwd = os.getcwd()
        if not (cwd + os.sep).startswith(os.path.realpath(tmp) + os.sep) and not (cwd + os.sep).startswith(tmp + os.sep):
            pre.add(tmp)
        pre.update(["/proc", "/sys", "/dev"])
        if OPTS.get("dir"):
            pre.add(OPTS["dir"])
        more = set()
        for p in pre:
            more.add(os.path.realpath(p))
        pre |= more
        self.excl = tuple(sorted(p.rstrip(os.sep) + os.sep for p in pre if p))
        self.own_prefixes = tuple(p + os.sep for p in self.own)
        self._install_audit()
        self._install_patches()
        self._install_imports()
        self._install_monitoring()
        self.on = True

    def _install_audit(self):
        rec = self
        handlers = {
            "open": self._on_open,
            "os.listdir": self._on_list,
            "os.scandir": self._on_list,
            "subprocess.Popen": lambda a: rec._unrec("runs a subprocess"),
            "os.system": lambda a: rec._unrec("runs a subprocess"),
            "os.posix_spawn": lambda a: rec._unrec("runs a subprocess"),
            "os.spawn": lambda a: rec._unrec("runs a subprocess"),
            "os.exec": lambda a: rec._unrec("runs a subprocess"),
            "os.fork": lambda a: rec._unrec("forks"),
            "os.forkpty": lambda a: rec._unrec("forks"),
            "socket.connect": self._on_connect,
            "socket.getaddrinfo": self._on_resolve,
            "socket.gethostbyname": self._on_resolve,
            "urllib.Request": lambda a: rec._unrec("uses the network"),
            "ctypes.dlopen": lambda a: rec._unrec_file("loads native code (ctypes)"),
            "sqlite3.connect": self._on_sqlite,
            "os.rename": self._on_mutate,
            "os.remove": self._on_mutate,
            "os.rmdir": self._on_mutate,
            "os.mkdir": self._on_mutate,
            "os.symlink": self._on_mutate,
            "os.link": self._on_mutate,
            "os.truncate": self._on_mutate,
            "os.chmod": self._on_mutate,
            "os.utime": self._on_mutate,
            "shutil.rmtree": self._on_mutate,
        }

        def audit(event, args):
            if not rec.on or rec.paused:
                return
            f = handlers.get(event)
            if f is not None:
                rec.paused += 1
                try:
                    f(args)
                except Exception as e:  # never break the test; never under-record
                    rec.cur.taint("recorder error on %s: %r" % (event, e))
                finally:
                    rec.paused -= 1

        sys.addaudithook(audit)

    def _unrec(self, why):
        self.cur.taint(why)

    def _unrec_file(self, why):
        if self.cur is not self.S:
            self.cur.taint(why)

    def _path(self, p):
        if isinstance(p, int) or p is None:
            return None
        if isinstance(p, bytes):
            p = os.fsdecode(p)
        else:
            p = os.fspath(p)
            if isinstance(p, bytes):
                p = os.fsdecode(p)
        if not os.path.isabs(p):
            p = os.path.join(os.getcwd(), p)
        return os.path.normpath(p)

    def _excluded(self, p):
        if "/__pycache__/" in p or p.endswith(".pyc"):
            return True
        return (p + os.sep).startswith(self.excl) or p.startswith(self.excl)

    def _own_key(self, p):
        if self.own_prefixes and p.startswith(self.own_prefixes):
            for d, name in self.own.items():
                if p.startswith(d + os.sep):
                    return "od:%s/%s" % (name, p[len(d) + 1 :])
        return None

    def _on_open(self, args):
        path, mode, flags = (tuple(args) + (None, None, None))[:3]
        p = self._path(path)
        if p is None:
            return
        if isinstance(mode, str):
            write = any(c in mode for c in "wax+")
        else:
            fl = flags if isinstance(flags, int) else 0
            write = bool(fl & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND))
        w = self.cur
        if write:
            self.written.add(p)
            self._on_write(p)
            return
        ok = self._own_key(p)
        if ok is not None:
            w.reads[ok] = None
            return
        if self._excluded(p) or p in self.written:
            return
        if "/.pytest_cache/" in p + "/":
            if w is not self.S:
                w.taint("reads pytest's cache (state from earlier runs)")
            return
        if self.loading and p.endswith(".py"):
            w.reads["m:" + p] = None
            return
        base = os.path.basename(p)
        if base in ("pyproject.toml", "tox.ini", "setup.cfg") and w is self.S:
            mod = _caller()
            if mod.startswith("_pytest") or mod.startswith("iniconfig") or mod == "tomllib" or mod.startswith("tomllib."):
                w.reads["ini:" + p] = None
                return
        w.reads["f:" + p] = None

    def _on_write(self, p):
        if self._excluded(p) or "/.pytest_cache/" in p + "/":
            return
        if p == WS or p.startswith(WS + os.sep):
            self.cur.taint("writes into the workspace (%s)" % rel(p))

    def _on_mutate(self, args):
        for a in args[:2]:
            p = self._path(a) if isinstance(a, (str, bytes, os.PathLike)) else None
            if p is not None:
                self.written.add(p)
                self._on_write(p)

    def _on_list(self, args):
        p = self._path(args[0] if args else ".")
        if p is None or self._excluded(p) or p in self.written:
            return
        mod = _caller()
        if mod.startswith("importlib") or mod.startswith("_frozen_importlib") or mod.startswith("_pytest") or mod.startswith("pluggy"):
            # Import resolution is keyed per imported name (x:); pytest's
            # collection finds new test files itself.
            return
        self.cur.reads["l:" + p] = None

    def _on_connect(self, args):
        addr = args[1] if len(args) > 1 else None
        if isinstance(addr, (str, bytes)):  # AF_UNIX
            return
        if isinstance(addr, tuple) and addr and addr[0] in _LOCALHOST:
            return
        self.cur.taint("uses the network")

    def _on_resolve(self, args):
        host = args[0] if args else None
        if isinstance(host, bytes):
            host = host.decode("ascii", "replace")
        if host in _LOCALHOST or host is None:
            return
        self.cur.taint("uses the network")

    def _on_sqlite(self, args):
        p = args[0] if args else None
        if isinstance(p, (str, bytes, os.PathLike)) and os.fspath(p) not in (":memory:", b":memory:", ""):
            q = self._path(p)
            if q and not self._excluded(q) and q not in self.written:
                self.cur.reads["f:" + q] = None

    # -- stats and env -----------------------------------------------------

    def _install_patches(self):
        rec = self

        def wrap(fn, label):
            def stat_like(path, *a, **kw):
                try:
                    return fn(path, *a, **kw)
                finally:
                    if rec.on and not rec.paused and kw.get("dir_fd") is None:
                        rec.paused += 1
                        try:
                            p = rec._path(path)
                            if p is not None:
                                ok = rec._own_key(p)
                                if ok is not None:
                                    rec.cur.reads[ok] = None
                                elif not rec._excluded(p) and p not in rec.written:
                                    rec.cur.reads["e:" + p] = None
                        except Exception:
                            pass
                        finally:
                            rec.paused -= 1

            stat_like.__wrapped__ = fn
            stat_like.__name__ = fn.__name__
            return stat_like

        for name in ("stat", "lstat", "access", "readlink"):
            orig = getattr(os, name, None)
            if orig is None:
                continue
            w = wrap(orig, name)
            for sset in ("supports_dir_fd", "supports_fd", "supports_follow_symlinks", "supports_effective_ids"):
                ss = getattr(os, sset, None)
                if isinstance(ss, set) and orig in ss:
                    ss.add(w)
            setattr(os, name, w)

        E = os._Environ
        gi, si, di, it = E.__getitem__, E.__setitem__, E.__delitem__, E.__iter__

        def _name(self, key):
            if isinstance(key, bytes):
                return os.fsdecode(key)
            return key if isinstance(key, str) else None

        def __getitem__(self, key):
            try:
                v = gi(self, key)
            except KeyError:
                rec._env(_name(self, key), None)
                raise
            rec._env(_name(self, key), v)
            return v

        def __setitem__(self, key, value):
            si(self, key, value)
            n = _name(self, key)
            if n is not None:
                rec.env_written.add(n)

        def __delitem__(self, key):
            di(self, key)
            n = _name(self, key)
            if n is not None:
                rec.env_written.add(n)

        def __iter__(self):
            if rec.on and not rec.paused:
                rec.cur.reads["venv:"] = None
            return it(self)

        E.__getitem__ = __getitem__
        E.__setitem__ = __setitem__
        E.__delitem__ = __delitem__
        E.__iter__ = __iter__

    def _env(self, name, value):
        if not self.on or self.paused or name is None or name in self.env_written:
            return
        if volatile(name):
            if self.cur is not self.S:
                self.cur.taint("reads the per-run variable %s" % name)
            elif _caller_ws():
                self.S.taint("the session reads the per-run variable %s" % name)
            return
        self.cur.reads["v:" + name] = None

    # -- imports -----------------------------------------------------------

    def _install_imports(self):
        import importlib._bootstrap as bootstrap
        import importlib._bootstrap_external as external

        rec = self
        orig_load = bootstrap._load_unlocked
        orig_gcd = bootstrap._gcd_import
        orig_import = builtins.__import__
        resolve = bootstrap._resolve_name
        calc_package = bootstrap._calc___package__
        orig_get_data = external.FileLoader.get_data

        def get_data(self, path):
            rec.loading += 1
            try:
                return orig_get_data(self, path)
            finally:
                rec.loading -= 1

        external.FileLoader.get_data = get_data

        def load_unlocked(spec):
            if not rec.on:
                return orig_load(spec)
            name = spec.name
            rec.cur.imports.update(_deps_of(name, ()))
            w = rec.loads.get(name)
            if w is None:
                w = rec.loads[name] = Window("l", name)
            rec.push(w)
            try:
                mod = orig_load(spec)
            finally:
                rec.pop()
            if GUARD is not None:
                GUARD.loaded(mod)
            return mod

        def gcd_import(name, package=None, level=0):
            try:
                mod = orig_gcd(name, package, level)
            except ImportError:
                if rec.on:
                    try:
                        rec.cur.failed.add(resolve(name, package, level) if level > 0 else name)
                    except Exception:
                        rec.cur.taint("import edge not resolved")
                raise
            if rec.on:
                try:
                    rec.cur.imports.update(_deps_of(resolve(name, package, level) if level > 0 else name, ()))
                except Exception:
                    rec.cur.taint("import edge not resolved")
            return mod

        def quod_import(name, globals=None, locals=None, fromlist=(), level=0):
            try:
                mod = orig_import(name, globals, locals, fromlist, level)
            except ImportError:
                if rec.on:
                    try:
                        absname = (resolve(name, calc_package(globals), level) if name else calc_package(globals)) if level > 0 else name
                        rec.cur.failed.add(absname)
                        if fromlist:
                            for f in fromlist:
                                if isinstance(f, str) and f != "*":
                                    rec.cur.failed.add(absname + "." + f)
                    except Exception:
                        rec.cur.taint("import edge not resolved")
                raise
            if rec.on:
                try:
                    absname = (resolve(name, calc_package(globals), level) if name else calc_package(globals)) if level > 0 else name
                    rec.cur.imports.update(_deps_of(absname, fromlist))
                    if fromlist:
                        for f in fromlist:
                            if isinstance(f, str) and f != "*" and absname + "." + f not in sys.modules:
                                # `from pkg import name`: maybe a submodule
                                # that does not exist yet.
                                rec.cur.failed.add(absname + "." + f)
                except Exception:
                    rec.cur.taint("import edge not resolved")
            return mod

        bootstrap._load_unlocked = load_unlocked
        bootstrap._gcd_import = gcd_import
        builtins.__import__ = quod_import

    def patch_rewrite(self):
        """pytest's assertion rewriter reads test modules and conftests
        itself: those reads are module loads (function-level narrowing
        applies), not data reads."""
        try:
            from _pytest.assertion import rewrite
        except Exception:
            return
        orig = rewrite._rewrite_test
        rec = self

        def _rewrite_test(fn, config):
            rec.loading += 1
            try:
                return orig(fn, config)
            finally:
                rec.loading -= 1

        rewrite._rewrite_test = _rewrite_test

    # -- executed functions (3.12+) -----------------------------------------

    def _install_monitoring(self):
        mon = getattr(sys, "monitoring", None)
        if mon is None or OPTS.get("no_narrow"):
            return
        tool = None
        for t in (4, 3, 2, 5):
            try:
                mon.use_tool_id(t, "quod")
                tool = t
                break
            except ValueError:
                continue
        if tool is None:
            return
        rec = self
        DISABLE = mon.DISABLE

        def on_start(code, offset):
            rec.stack[-1].exec.add(code)
            return DISABLE

        mon.register_callback(tool, mon.events.PY_START, on_start)
        mon.set_events(tool, mon.events.PY_START)
        self.mon = tool


def _caller_ws():
    f = sys._getframe(2)
    while f is not None:
        fn = f.f_code.co_filename
        if fn != _ME and (fn.startswith(WS + os.sep)) and "/site-packages/" not in fn:
            return True
        f = f.f_back
    return False


def _deps_of(absname, fromlist):
    names = set()
    parts = absname.split(".")
    for i in range(1, len(parts) + 1):
        names.add(".".join(parts[:i]))
    if fromlist:
        for f in fromlist:
            if isinstance(f, str) and f != "*":
                sub = absname + "." + f
                if sub in sys.modules:
                    names.add(sub)
    return names


# ---------------------------------------------------------------------------
# Shared state: the leak guard
#
# A fingerprint of state other test files can observe: every attribute of
# the workspace's modules (values to depth 2, class attributes), rebinding of
# every other module's attributes, interpreter-wide state (env, cwd, sys.path,
# warnings filters, logging, signal handlers, ...), session-scoped fixture
# values. A file's delta (across its collection, and across its tests) is
# recorded; the critical path compares the delta of every file it runs.

_SCALARS = (type(None), bool, int, float, complex, str, bytes)
_SKIP_ATTRS = {
    "__builtins__",
    "__loader__",
    "__spec__",
    "__cached__",
    "__file__",
    "__name__",
    "__package__",
    "__path__",
    "__doc__",
    "__warningregistry__",
    "@py_builtins",
    "@pytest_ar",
    "__annotations__",
}
_SYS_SKIP = {
    "stdout",
    "stderr",
    "stdin",
    "__stdout__",
    "__stderr__",
    "__stdin__",
    "last_type",
    "last_value",
    "last_traceback",
    "last_exc",
    "ps1",
    "ps2",
    "_",
    "exc_info",
    "argv",
    "modules",
    "path_importer_cache",
    "monitoring",
    "settrace",
    "setprofile",
}


def _tag(v, depth):
    t = type(v)
    try:
        if t in _SCALARS:
            r = repr(v) if not (t is str or t is bytes) or len(v) < 4096 else repr(v[:4096]) + "#%d" % len(v)
            return t.__name__ + ":" + hs(r)[:12]
        if isinstance(v, types.ModuleType):
            return "mod:" + v.__name__
        if isinstance(v, (types.FunctionType, types.BuiltinFunctionType, types.MethodType, staticmethod, classmethod, property)):
            return "fn:" + getattr(v, "__qualname__", t.__name__)
        if isinstance(v, type):
            return "cls:" + getattr(v, "__module__", "") + "." + v.__qualname__
        if isinstance(v, (dict, list, tuple, set, frozenset)) or t.__name__ in ("deque", "defaultdict", "OrderedDict", "Counter", "ChainMap"):
            n = len(v)
            if depth <= 0 or n > 200:
                return "%s#%d" % (t.__name__, n)
            if isinstance(v, dict):
                items = [(_tag(k, 0), _tag(x, depth - 1)) for k, x in list(v.items())]
            elif isinstance(v, (set, frozenset)):
                items = sorted(_tag(x, depth - 1) for x in list(v))
            else:
                items = [_tag(x, depth - 1) for x in list(v)]
            return "%s#%d:%s" % (t.__name__, n, hs(repr(items))[:12])
        d = getattr(v, "__dict__", None)
        if depth > 0 and isinstance(d, dict) and len(d) <= 200:
            items = [(k, _tag(x, depth - 1)) for k, x in sorted(list(d.items()), key=lambda kv: str(kv[0]))]
            return "obj:%s:%s" % (t.__qualname__, hs(repr(items))[:12])
        return "obj:" + t.__qualname__
    except Exception:
        return "obj:" + getattr(t, "__qualname__", "?")


def _is_ws_module(m):
    f = getattr(m, "__file__", None)
    if not isinstance(f, str):
        return False
    f = os.path.normpath(os.path.abspath(f))
    if not f.startswith(WS + os.sep) or f == _ME:
        return False
    return "/site-packages/" not in f and "/.venv/" not in f and not f.startswith(sys.prefix + os.sep)


def _module_fp(name, m, out):
    if _is_ws_module(m):
        for k, v in list(vars(m).items()):
            if k in _SKIP_ATTRS or isinstance(v, types.ModuleType):
                continue
            out["w:%s:%s" % (name, k)] = (id(v), _tag(v, 2))
            if isinstance(v, type) and getattr(v, "__module__", None) == name:
                for ck, cv in list(vars(v).items()):
                    if ck.startswith("__") and ck.endswith("__") or ck.startswith("_abc_"):
                        continue
                    out["w:%s:%s.%s" % (name, k, ck)] = (id(cv), _tag(cv, 1))
        return
    if m is sys.modules.get("__main__") or name in ("__main__", "quod_pytest", __name__) or name.startswith("_pytest") or name in ("pytest", "pluggy") or name.startswith("pluggy."):
        return
    try:
        items = list(vars(m).items())
    except TypeError:
        return
    skip = _SYS_SKIP if name == "sys" else ()
    for k, v in items:
        if k in _SKIP_ATTRS or k in skip or isinstance(v, types.ModuleType):
            continue
        if name == "builtins" and k in ("_", "__import__"):
            continue
        out["t:%s:%s" % (name, k)] = (id(v), type(v).__name__)


def fingerprint(session=None):
    out = {}
    for name, m in list(sys.modules.items()):
        if m is not None:
            _module_fp(name, m, out)
    g = out
    try:
        g["g:cwd"] = (0, hs(os.getcwd()))
    except OSError:
        g["g:cwd"] = (0, "gone")
    g["g:sys.path"] = (0, hs("\0".join(map(str, sys.path))))
    g["g:meta_path"] = (0, ",".join(type(x).__qualname__ for x in sys.meta_path))
    g["g:path_hooks"] = (0, ",".join(getattr(x, "__qualname__", type(x).__qualname__) for x in sys.path_hooks))
    for k, v in list(os.environ.items()):
        if not volatile(k):
            g["g:env:" + k] = (0, hs(v)[:12])
    import warnings

    g["g:warnings.filters"] = (0, _tag(list(warnings.filters), 1))
    import logging

    root = logging.getLogger()
    g["g:logging.root"] = (0, "%s:%s" % (root.level, ",".join(type(x).__qualname__ for x in root.handlers)))
    g["g:logging.disable"] = (0, str(logging.root.manager.disable))
    g["g:recursionlimit"] = (0, str(sys.getrecursionlimit()))
    g["g:switchinterval"] = (0, str(sys.getswitchinterval()))
    g["g:tzname"] = (0, repr(time.tzname))
    import gc

    g["g:gc"] = (0, "%s:%s" % (gc.isenabled(), gc.get_threshold()))
    if "decimal" in sys.modules:
        try:
            g["g:decimal"] = (0, hs(repr(sys.modules["decimal"].getcontext())))
        except Exception:
            pass
    if "socket" in sys.modules:
        g["g:socket.timeout"] = (0, repr(sys.modules["socket"].getdefaulttimeout()))
    if "locale" in sys.modules:
        try:
            g["g:locale"] = (0, sys.modules["locale"].setlocale(sys.modules["locale"].LC_ALL))
        except Exception:
            pass
    try:
        import signal

        for s in ("SIGINT", "SIGTERM", "SIGALRM", "SIGUSR1", "SIGUSR2", "SIGHUP"):
            if hasattr(signal, s):
                g["g:signal." + s] = (0, _tag(signal.getsignal(getattr(signal, s)), 0))
    except Exception:
        pass
    g["g:threads"] = (0, str(sorted(t.name for t in threading.enumerate() if t is not threading.main_thread())))
    if session is not None:
        try:
            fm = session._fixturemanager
            for fname, defs in list(fm._arg2fixturedefs.items()):
                for d in defs:
                    sc = getattr(d.scope, "value", d.scope)
                    cr = getattr(d, "cached_result", None)
                    if sc in ("session", "package") and cr is not None:
                        out["fx:%s" % fname] = (id(cr[0]), _tag(cr[0], 2))
        except Exception:
            pass
    return out


class Guard:
    """Fingerprints at file boundaries; the delta of a window."""

    def __init__(self):
        self.before = None
        self.extra = {}  # state of modules first loaded inside the window, at load

    def start(self, session):
        _pause(1)
        try:
            self.before = fingerprint(session)
        finally:
            _pause(-1)
        self.extra = {}

    def loaded(self, mod):
        if self.before is None or mod is None:
            return
        _pause(1)
        try:
            out = {}
            _module_fp(mod.__name__, mod, out)
            for k, v in out.items():
                if k not in self.before:
                    self.extra.setdefault(k, v)
        except Exception:
            pass
        finally:
            _pause(-1)

    def fixture_set(self, name, value):
        if self.before is not None:
            self.extra.setdefault("fx:" + name, (id(value), _tag(value, 2)))

    def end(self, session):
        _pause(1)
        try:
            after = fingerprint(session)
        finally:
            _pause(-1)
        d = {}
        before = dict(self.extra)
        before.update(self.before or {})
        for k, v in after.items():
            b = before.get(k)
            if b is None:
                if k.startswith("t:"):
                    continue  # a third-party module's new attribute: lazy caches
                d[k] = v[1]
            elif b != v:
                if k.startswith("t:") and b[1] == "NoneType":
                    continue  # lazy initialisation of a third-party global
                d[k] = v[1]
        for k in before:
            if k not in after and not k.startswith("t:") and not k.startswith("fx:"):
                d[k] = "deleted"
        self.before = after
        self.extra = {}
        return d


GUARD = None


def _pause(n):
    if REC is not None:
        REC.paused += n


def leak_relevant(key, mods, fixtures):
    """Can a test file whose import closure is `mods` (None: unknown) and
    whose fixtures are `fixtures` observe this key?"""
    if key.startswith("g:"):
        return True
    if mods is None:
        return True
    if key.startswith("w:") or key.startswith("t:"):
        mod = key.split(":", 2)[1]
        return mod in mods
    if key.startswith("fx:"):
        return fixtures is None or key[3:] in fixtures
    return True


# ---------------------------------------------------------------------------
# Function-level narrowing


def _first_line(fn):
    lines = [d.lineno for d in fn.decorator_list] + [fn.lineno]
    return min(lines)


def _functions(tree):
    """[(qualpath, firstline, name, body_start, body_end)] for every def."""
    out = []

    def visit(node, path):
        for child in ast.iter_child_nodes(node):
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                body = child.body
                start = body[0]
                if (
                    isinstance(start, ast.Expr)
                    and isinstance(getattr(start, "value", None), ast.Constant)
                    and isinstance(start.value.value, str)
                ):
                    # The docstring is the function's __doc__ (doctests,
                    # introspection): a change there is not narrowable.
                    if len(body) == 1:
                        visit(child, path + (child.name,))
                        continue
                    start = body[1]
                out.append((path + (child.name,), _first_line(child), child.name, start.lineno, child.end_lineno))
                visit(child, path + (child.name,))
            elif isinstance(child, ast.ClassDef):
                visit(child, path + (child.name,))
            else:
                visit(child, path)

    visit(tree, ())
    return out


def _innermost(funcs, lo, hi, insertion):
    """The innermost def whose body holds lines [lo, hi] (1-based, inclusive);
    for an insertion between lines lo-1 and lo: one whose body spans it."""
    best = None
    for f in funcs:
        _, _, _, bs, be = f
        if insertion:
            ok = bs <= lo - 1 and lo - 1 <= be and lo <= be + 1 and lo - 1 >= bs
            # lines inserted after line lo-1: inside when lo-1 is a body line
            # and lo-1 < be (strictly before the body's end) or the new side
            # agrees (checked separately)
            ok = bs <= lo - 1 <= be
        else:
            ok = bs <= lo and hi <= be
        if ok and (best is None or (f[3] >= best[3] and f[4] <= best[4])):
            best = f
    return best


def narrowable(old_src, new_src, executed):
    """True when every change from old_src to new_src lies strictly inside the
    body of a function never executed (executed: {(firstline, name)} of the
    old version)."""
    try:
        old_tree = ast.parse(old_src)
        new_tree = ast.parse(new_src)
        compile(new_src, "<quod>", "exec")
    except Exception:
        return False
    of = _functions(old_tree)
    nf = _functions(new_tree)
    a = old_src.splitlines()
    b = new_src.splitlines()
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    any_change = False
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        any_change = True
        fo = _innermost(of, i1 + 1, i2, i1 == i2)
        fn = _innermost(nf, j1 + 1, j2, j1 == j2)
        if fo is None or fn is None or fo[0] != fn[0]:
            return False
        if (fo[1], fo[2]) in executed:
            return False
        # Insertions at the very end of a body: the new side must agree the
        # lines are inside (a dedented statement would not be).
        if i1 == i2 and not (fn[3] <= j1 + 1 and j2 <= fn[4]):
            return False
        if j1 == j2 and not (fo[3] <= i1 + 1 and i2 <= fo[4]):
            return False
    return any_change


# ---------------------------------------------------------------------------
# Reports


def _strip(data):
    data = dict(data)
    for k in [k for k in data if k.startswith("_hypothesis")]:
        data.pop(k)
    return data


_DEFAULTS = {"longrepr": None, "sections": [], "user_properties": []}


def compact(reports):
    """A test's serialized reports without what replay can rebuild: the
    report type, node id and location (the test's own), defaults, and the
    keywords after the first phase's."""
    kw = None
    out = []
    for r in reports:
        r = dict(r)
        k = r.pop("keywords", None)
        if kw is None:
            kw = k
        elif k != kw:
            r["keywords"] = k
        for f in ("$report_type", "nodeid", "location"):
            r.pop(f, None)
        for f, d in _DEFAULTS.items():
            if f in r and r[f] == d:
                r.pop(f)
        out.append(r)
    return kw, out


def expand(t):
    reps = []
    for r in t["reports"]:
        d = {"$report_type": "TestReport", "nodeid": t["nodeid"], "location": t["location"], "keywords": t.get("keywords") or {}}
        d.update(_DEFAULTS)
        d["sections"] = []
        d["user_properties"] = []
        d.update(r)
        if isinstance(d.get("longrepr"), list):
            d["longrepr"] = tuple(d["longrepr"])
        reps.append(d)
    return reps


def outcome_of(reports):
    """One test's result, from its serialized reports."""
    for r in reports:
        if r.get("outcome") == "failed":
            if r.get("wasxfail") is not None and r.get("when") == "call":
                return "xpassed-strict"
            return "failed:%s" % r.get("when")
    for r in reports:
        if r.get("wasxfail") is not None:
            return "xfailed" if r.get("outcome") == "skipped" else "xpassed"
    for r in reports:
        if r.get("outcome") == "skipped":
            lr = r.get("longrepr")
            reason = lr[2] if isinstance(lr, (list, tuple)) and len(lr) == 3 else ""
            return "skipped:%s" % reason
    return "passed"


def _warning_category(qualname):
    mod, _, name = qualname.rpartition(".")
    if not mod:
        return getattr(builtins, name, None)
    m = sys.modules.get(mod)
    if m is None:
        try:
            __import__(mod)
            m = sys.modules.get(mod)
        except Exception:
            return None
    obj = m
    for part in name.split("."):
        obj = getattr(obj, part, None)
    return obj if isinstance(obj, type) and issubclass(obj, Warning) else None


# ---------------------------------------------------------------------------
# The ledger


def ledger_path():
    return os.path.join(OPTS["dir"], "ledger.json")


def load_ledger():
    try:
        with open(ledger_path()) as f:
            L = json.load(f)
    except (OSError, ValueError):
        return None
    if L.get("version") != LEDGER_VERSION:
        return None
    return L


def load_results(fid):
    try:
        with open(os.path.join(OPTS["dir"], "results", fid + ".json")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def file_id(relpath):
    return hs(relpath)[:20]


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp.%d" % os.getpid()
    with open(tmp, "w") as f:
        json.dump(obj, f, separators=(",", ":"))
    os.replace(tmp, path)


_OUT_FLAGS = {
    "-v",
    "-vv",
    "-vvv",
    "-q",
    "-qq",
    "--verbose",
    "--quiet",
    "-l",
    "--showlocals",
    "--no-showlocals",
    "--no-header",
    "--no-summary",
    "--full-trace",
    "-ra",
    "-rA",
}
_OUT_VALUED = ("--junitxml", "--junit-xml", "--junit-prefix", "--color", "--code-highlight", "--durations", "--durations-min", "--tb", "-r")


def keyed_args(args):
    """pytest's arguments that change what a test does (output-only ones are
    not keyed: replays render through whatever this run asks for)."""
    out = []
    i = 0
    while i < len(args):
        a = args[i]
        if a in _OUT_FLAGS:
            i += 1
            continue
        hit = False
        for v in _OUT_VALUED:
            if a == v:
                i += 2
                hit = True
                break
            if a.startswith(v + "=") or (v == "-r" and a.startswith("-r") and len(a) > 2 and not a.startswith("--")):
                i += 1
                hit = True
                break
        if hit:
            continue
        out.append(a)
        i += 1
    return out


_UNSAFE_PLUGINS = ("randomly", "random_order", "rerunfailures", "forked", "flaky", "testmon", "picked", "xdist.looponfail")


def unsafe_reason(config):
    opt = config.option
    if getattr(opt, "maxfail", 0):
        return "--maxfail/-x stops on a failure"
    for name in ("lf", "failedfirst", "newfirst", "stepwise", "stepwise_skip", "usepdb", "trace", "looponfail"):
        if getattr(opt, name, False):
            return "--%s changes which tests run or in what order" % name
    for name in ("collectonly", "setuponly", "setupplan"):
        if getattr(opt, name, False):
            return "--%s runs no tests" % name
    if getattr(opt, "reruns", 0):
        return "--reruns"
    if getattr(opt, "count", 1) not in (None, 1):
        return "--count"
    n = getattr(opt, "numprocesses", None)
    if n not in (None, 0, "0"):
        return "pytest-xdist (-n) is not supported yet"
    for p in _UNSAFE_PLUGINS:
        if config.pluginmanager.hasplugin(p):
            if p == "randomly" and getattr(opt, "randomly_seed", None) is not None and getattr(opt, "randomly_dont_reorganize", False):
                continue
            return "plugin %s changes order or reruns tests" % p
    return None


def tool_state(config):
    import pytest

    plugins = []
    for p in config.pluginmanager.get_plugins():
        if p is None or isinstance(p, Quod):
            continue
        f = getattr(p, "__file__", None)
        if isinstance(f, str) and os.path.abspath(f).startswith(WS + os.sep) and "/site-packages/" not in f:
            continue  # conftests: keyed as files
        plugins.append(getattr(p, "__name__", None) or type(p).__name__)
    plugins = sorted(set(plugins))
    return hs(json.dumps([PLUGIN_HASH, pytest.__version__, sys.version, plugins]))


# ---------------------------------------------------------------------------
# The pytest plugin


def hookimpl(**opts):
    """pytest's hookimpl marker without importing pytest (the recorder must
    be installed before pytest imports anything of the workspace)."""

    def deco(f):
        o = {"wrapper": False, "hookwrapper": False, "optionalhook": False, "tryfirst": False, "trylast": False, "specname": None}
        o.update(opts)
        f.pytest_impl = o
        return f

    return deco


def _make_replay_item_class():
    import pytest

    class ReplayItem(pytest.Item):
        def __init__(self, *, quod_rec, quod_loc, **kw):
            super().__init__(**kw)
            self.quod_rec = quod_rec
            self._quod_loc = tuple(quod_loc)

        @property
        def location(self):
            return self._quod_loc

        def reportinfo(self):
            return (self.path, self._quod_loc[1], self._quod_loc[2])

        def runtest(self):
            raise RuntimeError("quod: a replayed test was executed")

    return ReplayItem


class _Deselected:
    def __init__(self, nodeid):
        self.nodeid = nodeid
        self.quod_placeholder = True


def _sortkey(relpath):
    return tuple(relpath.split("/"))


class Quod:
    def __init__(self, mode, args):
        self.mode = mode
        self.args = args
        self.config = None
        self.session = None
        self.L = None
        self.taint = None
        self.replay = {}  # rel -> record (files replayed)
        self.collect_only = set()  # replayed, but collected (their collection leaks)
        self.ignore = set()
        self.run_files = set()
        self.reasons = {}
        self.narrowed = []
        self.pos = {}
        self.order = []
        self.audit = set()
        self.items_by_file = {}  # rel -> [nodeid]
        self.locs = {}
        self.file_of = {}
        self.results = {}  # nodeid -> {"reports": [...], "warnings": [...]}
        self.collect_warnings = {}
        self.collect_status = {}
        self.deselected = {}
        self.cleak = {}
        self.rleak = {}
        self.fixtures = {}
        self.replayed_tests = 0
        self.replayed_files = []
        self.tripped = None
        self.select_ms = 0
        self.collected_files = []
        self.cur_collect = None
        self.cur_run = None
        self.unexpected = []
        self.ReplayItem = None
        self.states = None
        self.changed_keys = {}
        self.session_changed = []
        self.guard_on = False
        self.tool = None
        self.thread_mark = None

    # -- configuration and selection ---------------------------------------

    @hookimpl(trylast=True)
    def pytest_configure(self, config):
        global WS
        self.config = config
        WS = str(config.rootpath)
        self.taint = unsafe_reason(config)
        if REC is not None:
            REC.patch_rewrite()

    def pytest_sessionstart(self, session):
        self.session = session
        self.ReplayItem = _make_replay_item_class()
        self.tool = tool_state(self.config)
        t0 = time.monotonic()
        if REC is not None:
            REC.paused += 1
        try:
            if self.mode in ("critical", "shadow"):
                self.select()
            elif self.mode == "alone":
                self.ignore = set(OPTS.get("ignore", [])) - {OPTS["file"]}
        finally:
            if REC is not None:
                REC.paused -= 1
        self.select_ms = int((time.monotonic() - t0) * 1000)
        global GUARD
        if self.mode == "shadow" or (self.mode == "critical" and self.replay):
            GUARD = Guard()
            self.guard_on = True
            if self.mode == "critical":
                self._install_load_hook()

    def _install_load_hook(self):
        import importlib._bootstrap as bootstrap

        orig = bootstrap._load_unlocked

        def load_unlocked(spec):
            mod = orig(spec)
            if GUARD is not None:
                GUARD.loaded(mod)
            return mod

        bootstrap._load_unlocked = load_unlocked

    def select(self):
        L = load_ledger()
        if L is None:
            self.reasons["*"] = "no ledger (the first run)"
            return
        if OPTS.get("full"):
            self.reasons["*"] = "--full"
            return
        if self.taint:
            self.reasons["*"] = self.taint
            return
        if L.get("rootdir") != WS:
            self.reasons["*"] = "another rootdir"
            return
        if L.get("args") != keyed_args(self.args):
            self.reasons["*"] = "other pytest arguments"
            return
        if L.get("tool") != self.tool:
            self.reasons["*"] = "pytest, its plugins, Python or quod changed"
            return
        if L["session"].get("unrecordable"):
            self.reasons["*"] = "session unrecordable: %s" % L["session"]["unrecordable"]
            return
        self.L = L
        st = self.states = States(L.get("stat"))
        sess_changed = [k for k, v in L["session"]["keys"].items() if st.state(k) != v]
        hard = [k for k in sess_changed if not k.startswith("m:")]
        self.session_changed = sess_changed
        if hard:
            self.reasons["*"] = "shared inputs changed: %s" % ", ".join(hard[:5])
            self.L = None
            return
        sess_exec = L["session"].get("exec", {})
        files = L["files"]
        # The full run's order: recorded, minus deleted files, plus new files
        # a pre-scan predicts.
        order = [f for f in L["order"] if f in files and os.path.exists(absp(f))]
        known = set(L["order"])
        new = []
        pats = L.get("collect", {}).get("patterns", ["test_*.py", "*_test.py"])
        for d in L.get("collect", {}).get("dirs", []):
            for n in st.listing(absp(d)) or ():
                r = n if d == "." else d + "/" + n
                if r in known:
                    continue
                if any(fnmatch.fnmatch(n, p) for p in pats) and os.path.isfile(absp(r)):
                    new.append(r)
        for r in sorted(new, key=_sortkey):
            k = _sortkey(r)
            at = len(order)
            for i, f in enumerate(order):
                if _sortkey(f) > k:
                    at = i
                    break
            order.insert(at, r)
        self.order = order
        self.pos = {f: i for i, f in enumerate(order)}
        run = set(new)
        for r in new:
            self.reasons[r] = "new"
        replay = {}
        for f in order:
            if f in run:
                continue
            r = files[f]
            why = None
            if r.get("quarantined"):
                why = "quarantined: %s" % r["quarantined"]
            elif r.get("unrecordable"):
                why = "unrecordable: %s" % r["unrecordable"]
            elif r.get("status") != "passed":
                why = "recorded %s" % r.get("status")
            elif r.get("certified") is not True:
                why = "not certified order-independent"
            if why is None:
                changed = [k for k, v in r["keys"].items() if st.state(k) != v]
                changed += [k for k in sess_changed if k not in r["keys"]]
                if changed:
                    hard = [k for k in changed if not k.startswith("m:")]
                    if hard:
                        why = "changed: %s" % ", ".join(hard[:3])
                    elif OPTS.get("no_narrow") or not self._narrow(r, changed, sess_exec):
                        why = "changed: %s" % ", ".join(changed[:3])
                    else:
                        self.narrowed.append(f)
                self.changed_keys[f] = changed
            if why is None:
                replay[f] = r
            else:
                run.add(f)
                self.reasons[f] = why
        # Cross-file state. A replayed file's leaks (recorded deltas of what
        # other files can observe) must still happen for every file that runs:
        # - a replayed file whose collection leaks is collected (its tests
        #   still replay);
        # - a replayed file before a running one whose tests leak runs;
        # - a running file known to be order-dependent runs with its prefix.
        collect = set()
        changed = True
        while changed:
            changed = False
            if not run:
                break
            for f in list(replay):
                r = replay[f]
                if r.get("cleak") and f not in collect:
                    if any(leak_relevant(k, None, None) for k in r["cleak"]):
                        collect.add(f)
                        changed = True
                if r.get("rleak") and any(self.pos[a] > self.pos[f] for a in run):
                    run.add(f)
                    replay.pop(f)
                    collect.discard(f)
                    self.reasons[f] = "its tests leak state into a later file that runs"
                    changed = True
            for a in list(run):
                ra = files.get(a)
                if ra is not None and ra.get("certified") is False:
                    for f in list(replay):
                        if self.pos[f] < self.pos[a]:
                            run.add(f)
                            replay.pop(f)
                            collect.discard(f)
                            self.reasons[f] = "before %s, which is order-dependent" % a
                            changed = True
        self.run_files = run
        self.replay = replay
        self.collect_only = collect
        self.ignore = set(replay) - collect
        if self.mode == "shadow":
            self._pick_audits()

    def _narrow(self, r, changed, sess_exec):
        exe = r.get("exec", {})
        quotes = set(r.get("quotes", []))
        for k in changed:
            p = k[2:]
            if p in quotes:
                return False
            old = r["keys"].get(k) or self.L["session"]["keys"].get(k)
            if not old or len(old) != 32:
                return False
            try:
                with open(os.path.join(OPTS["dir"], "blobs", old), "rb") as f:
                    old_src = f.read().decode("utf-8")
                with open(absp(p), "rb") as f:
                    new_src = f.read().decode("utf-8")
            except (OSError, UnicodeDecodeError):
                return False
            executed = set(tuple(x) for x in exe.get(p, ())) | set(tuple(x) for x in sess_exec.get(p, ()))
            if not narrowable(old_src, new_src, executed):
                return False
        return True

    def _pick_audits(self):
        n = int(OPTS.get("audits", 0) or 0)
        if n <= 0 or not self.replay:
            return
        cands = sorted(self.replay, key=lambda f: (self.replay[f].get("audited", 0), f))
        for f in cands[:n]:
            self.audit.add(f)
            self.replay.pop(f)
            self.collect_only.discard(f)
            self.ignore.discard(f)
            self.run_files.add(f)
            self.reasons[f] = "audit"

    # -- collection ----------------------------------------------------------

    def pytest_ignore_collect(self, collection_path, config):
        if not self.ignore:
            return None
        r = rel(str(collection_path))
        if r in self.ignore:
            return True
        return None

    @hookimpl(hookwrapper=True)
    def pytest_make_collect_report(self, collector):
        import pytest

        is_file = isinstance(collector, pytest.File)
        r = rel(str(collector.path)) if is_file else None
        if is_file:
            self.cur_collect = r
            if REC is not None:
                REC.push(REC.file(r))
            if self.guard_on:
                GUARD.start(self.session)
        try:
            outcome = yield
        finally:
            if is_file:
                if self.guard_on:
                    self.cleak[r] = GUARD.end(self.session)
                if REC is not None:
                    REC.pop()
                self.cur_collect = None
        if is_file:
            try:
                rep = outcome.get_result()
                self.collect_status[r] = rep.outcome
                if r not in self.items_by_file:
                    self.items_by_file[r] = []
                    self.collected_files.append(r)
            except Exception:
                self.collect_status[r] = "failed"

    def pytest_deselected(self, items):
        for it in items:
            if getattr(it, "quod_placeholder", False):
                continue
            p = getattr(it, "path", None)
            if p is not None:
                r = rel(str(p))
                self.deselected[r] = self.deselected.get(r, 0) + 1

    @hookimpl(trylast=True)
    def pytest_collection_modifyitems(self, session, config, items):
        for it in items:
            r = rel(str(it.path))
            self.file_of[it.nodeid] = r
            self.items_by_file.setdefault(r, [])
            if r not in self.collected_files:
                self.collected_files.append(r)
            self.items_by_file[r].append(it.nodeid)
            self.locs[it.nodeid] = list(it.location)
            if REC is not None or self.mode != "critical":
                self.fixtures.setdefault(r, set()).update(getattr(it, "fixturenames", ()))
        if self.mode == "critical" and self.L is not None:
            for r in self.collected_files:
                if r not in self.run_files and r not in self.collect_only and r not in self.replay:
                    self.unexpected.append(r)
            if self.unexpected and any(rr.get("cleak") or rr.get("rleak") for rr in self.replay.values()):
                self.trip("test files the pre-scan did not predict (%s) beside replayed files that leak state" % ", ".join(self.unexpected[:3]))
        if self.mode == "critical" and self.replay and self.guard_on:
            self._check_collect_leaks()
        if not self.replay:
            return
        # Replace the collected-only files' items, add the replayed ones, in
        # the full run's order.
        keep = [it for it in items if rel(str(it.path)) not in self.replay]
        groups = []  # (pos, seq, [items])
        seq = 0
        cur = None
        for it in keep:
            r = rel(str(it.path))
            if cur is None or cur[0] != r:
                cur = (r, [])
                groups.append((self._pos(r), seq, cur[1]))
                seq += 1
            cur[1].append(it)
        n_replay = 0
        n_desel = 0
        tr = config.pluginmanager.get_plugin("terminalreporter")
        for f, rec in self.replay.items():
            res = load_results(rec["id"])
            if res is None:
                self.trip("the recorded results of %s are missing" % f)
                return
            made = []
            for t in res["items"]:
                it = self.ReplayItem.from_parent(
                    session,
                    name=t["nodeid"].rsplit("::", 1)[-1],
                    nodeid=t["nodeid"],
                    path=__import__("pathlib").Path(absp(f)),
                    quod_rec=t,
                    quod_loc=t["location"],
                )
                made.append(it)
            groups.append((self._pos(f), seq, made))
            seq += 1
            n_replay += len(made)
            if res.get("deselected") and f not in self.collect_only:
                n_desel += res["deselected"]
                config.hook.pytest_deselected(items=[_Deselected("%s::<deselected>" % f)] * res["deselected"])
            self.replayed_files.append(f)
        groups.sort(key=lambda g: (g[0], g[1]))
        items[:] = [it for g in groups for it in g[2]]
        if tr is not None and hasattr(tr, "_numcollected"):
            tr._numcollected += n_replay + n_desel - sum(len(self.items_by_file.get(f, [])) for f in self.collect_only)

    def _pos(self, r):
        p = self.pos.get(r)
        if p is not None:
            return p
        # Unpredicted: between its neighbours by path.
        k = _sortkey(r)
        for f in self.order:
            if _sortkey(f) > k:
                return self.pos[f] - 0.5
        return len(self.order) + 0.5

    # -- running ---------------------------------------------------------------

    @hookimpl(tryfirst=True)
    def pytest_runtest_protocol(self, item, nextitem):
        if self.ReplayItem is None or not isinstance(item, self.ReplayItem):
            return None
        rec = item.quod_rec
        config = item.config
        ihook = item.ihook
        ihook.pytest_runtest_logstart(nodeid=item.nodeid, location=item.location)
        for data in expand(rec):
            rep = config.hook.pytest_report_from_serializable(config=config, data=data)
            rep.quod_replayed = True
            ihook.pytest_runtest_logreport(report=rep)
        for w in rec.get("warnings", []):
            import warnings

            cat = _warning_category(w["category"]) or UserWarning
            msg = warnings.WarningMessage(cat(w["message"]), cat, w["filename"], w["lineno"])
            config.hook.pytest_warning_recorded.call_historic(
                kwargs=dict(warning_message=msg, when="runtest", nodeid=item.nodeid, location=None)
            )
        ihook.pytest_runtest_logfinish(nodeid=item.nodeid, location=item.location)
        self.replayed_tests += 1
        return True

    def _run_wrapper(self, item, nextitem):
        if self.ReplayItem is not None and isinstance(item, self.ReplayItem):
            yield
            return
        r = self.file_of.get(item.nodeid) or rel(str(item.path))
        first = self.cur_run != r
        if first:
            self.cur_run = r
            if self.guard_on:
                GUARD.start(self.session)
            if REC is not None:
                self.thread_mark = set(threading.enumerate())
        if REC is not None:
            REC.push(REC.file(r))
        try:
            yield
        finally:
            if REC is not None:
                REC.pop()
            nr = None
            if nextitem is not None and not (self.ReplayItem is not None and isinstance(nextitem, self.ReplayItem)):
                nr = self.file_of.get(nextitem.nodeid) or rel(str(nextitem.path))
            if nr != r:
                self.cur_run = None
                if REC is not None and self.thread_mark is not None:
                    left = [t for t in threading.enumerate() if t not in self.thread_mark and t.is_alive()]
                    if left:
                        REC.file(r).taint("a thread it started outlived it (%s)" % left[0].name)
                if self.guard_on:
                    d = GUARD.end(self.session)
                    self.rleak[r] = d
                    if self.mode == "critical":
                        self._check_leak(r, d)

    @hookimpl(hookwrapper=True)
    def pytest_fixture_setup(self, fixturedef, request):
        sc = getattr(fixturedef.scope, "value", fixturedef.scope)
        shared = sc in ("session", "package")
        if shared and REC is not None:
            REC.push(REC.S)
        try:
            outcome = yield
        finally:
            if shared and REC is not None:
                REC.pop()
        if shared and self.guard_on:
            try:
                GUARD.fixture_set(fixturedef.argname, outcome.get_result())
            except BaseException:
                pass

    def pytest_runtest_logreport(self, report):
        if getattr(report, "quod_replayed", False):
            return
        t = self.results.setdefault(report.nodeid, {"reports": [], "warnings": []})
        if self.mode == "shadow":
            data = self.config.hook.pytest_report_to_serializable(config=self.config, report=report)
            t["reports"].append(_strip(data))
        else:
            t["reports"].append(
                {
                    "outcome": report.outcome,
                    "when": report.when,
                    "wasxfail": getattr(report, "wasxfail", None),
                    "longrepr": list(report.longrepr) if isinstance(report.longrepr, tuple) else None,
                }
            )

    def pytest_warning_recorded(self, warning_message, when, nodeid, location):
        if self.mode != "shadow":
            return
        cat = warning_message.category
        w = {
            "category": "%s.%s" % (cat.__module__, cat.__qualname__) if cat.__module__ != "builtins" else cat.__qualname__,
            "message": str(warning_message.message),
            "filename": warning_message.filename,
            "lineno": warning_message.lineno,
        }
        if when == "runtest" and nodeid:
            self.results.setdefault(nodeid, {"reports": [], "warnings": []})["warnings"].append(w)
        elif when == "collect" and self.cur_collect:
            self.collect_warnings.setdefault(self.cur_collect, []).append(w)

    @hookimpl(hookwrapper=True)
    def pytest_runtestloop(self, session):
        if REC is not None and REC.thread_start is None:
            REC.thread_start = set(threading.enumerate())
        outcome = yield
        if REC is not None:
            REC.on = False
        return outcome

    def _check_leak(self, r, d):
        """The critical path's guard: a file that ran here changed state a
        replayed file can observe, differently from when it was recorded."""
        if self.tripped or not d:
            return
        rec = self.L["files"].get(r) if self.L else None
        recorded = (rec or {}).get("rleak") or {}
        here = self._pos(r)
        later = [self.replay[f] for f in self.replay if self._pos(f) > here]
        for k, v in d.items():
            if recorded.get(k) == v:
                continue
            for b in later:
                if leak_relevant(k, set(b.get("modules", ())), set(b.get("fixtures", ()))):
                    self.trip("%s changed shared state %s (a replayed later file can observe it)" % (r, k))
                    return

    def _check_collect_leaks(self):
        for r, cd in self.cleak.items():
            if not cd:
                continue
            rec = self.L["files"].get(r) or {}
            crec = rec.get("cleak") or {}
            for k, v in cd.items():
                if crec.get(k) == v:
                    continue
                for f, b in self.replay.items():
                    if f != r and leak_relevant(k, set(b.get("modules", ())), set(b.get("fixtures", ()))):
                        self.trip("collecting %s changed shared state %s" % (r, k))
                        return

    def trip(self, why):
        if self.tripped is None:
            self.tripped = why

    def pytest_terminal_summary(self, terminalreporter):
        if self.replayed_tests:
            terminalreporter.write_line(
                "quod: %d tests in %d files replayed (their inputs did not change)" % (self.replayed_tests, len(self.replayed_files))
            )
        if self.tripped and self.mode == "critical":
            terminalreporter.write_line("quod: %s; running the full suite" % self.tripped)

    # -- outcomes -------------------------------------------------------------

    def outcomes(self):
        out = {}
        for nodeid, t in self.results.items():
            out[nodeid] = outcome_of(t["reports"])
        return out


def _register(plugin):
    # pluggy keys hooks by attribute name; the wrapper needs its own name.
    import pytest

    class Wrap:
        @pytest.hookimpl(hookwrapper=True)
        def pytest_runtest_protocol(self, item, nextitem):
            yield from plugin._run_wrapper(item, nextitem)

    return Wrap()


# ---------------------------------------------------------------------------
# The shadow's recording


def closure(start, loads):
    seen = set()
    stack = list(start)
    while stack:
        m = stack.pop()
        if m in seen:
            continue
        seen.add(m)
        w = loads.get(m)
        if w is not None:
            stack.extend(w.imports)
    return seen


def _code_map(codes):
    out = {}
    for c in codes:
        fn = c.co_filename
        if not fn or fn.startswith("<"):
            continue
        p = os.path.normpath(os.path.abspath(fn))
        if not p.startswith(WS + os.sep) or p == _ME or "/site-packages/" in p:
            continue
        out.setdefault(rel(p), set()).add((c.co_firstlineno, c.co_name))
    return {k: sorted(v) for k, v in out.items()}


def window_keys(windows, names, st):
    keys = {}
    for w in windows:
        for k in w.reads:
            kind, _, p = k.partition(":")
            if kind in ("f", "m", "e", "l", "ini"):
                k = kind + ":" + rel(p)
            keys[k] = None
    # Import resolution: every name tried, in each workspace directory it
    # was (or would be) searched in.
    ws_path = []
    for p in sys.path:
        p = os.path.normpath(os.path.abspath(p or "."))
        if (p == WS or p.startswith(WS + os.sep)) and os.path.isdir(p) and p not in ws_path:
            ws_path.append(p)
    for n in names:
        if "." not in n:
            dirs = ws_path
            stem = n
        else:
            parent, _, stem = n.rpartition(".")
            pm = sys.modules.get(parent)
            dirs = []
            for p in list(getattr(pm, "__path__", None) or ()):
                p = os.path.normpath(os.path.abspath(p))
                if p == WS or p.startswith(WS + os.sep):
                    dirs.append(p)
        for d in dirs:
            keys["x:%s/%s" % (rel(d), stem)] = None
    return {k: st.state(k) for k in keys}


def record_shadow(plugin, rc):
    """After the shadow's session: record every file that ran and whose
    results match the critical path's; certify; write the ledger."""
    t0 = time.monotonic()
    REC.on = False
    REC.paused += 1
    summary = {"recorded": 0, "certified": 0, "uncertified": [], "quarantined": [], "unrecordable": {}, "audit_mismatches": [], "not_recorded": {}}
    if rc not in (0, 1, 5):
        summary["error"] = "pytest exited %d: nothing recorded" % rc
        return summary
    L = load_ledger() if plugin.L is None else plugin.L
    if L is not None and (L.get("rootdir") != WS or L.get("args") != keyed_args(plugin.args) or L.get("tool") != plugin.tool):
        L = None
    if L is None:
        L = {"version": LEDGER_VERSION, "files": {}, "order": [], "seq": 0, "session": {"keys": {}}}
    st = States(L.get("stat"), REC.own)
    L["seq"] = L.get("seq", 0) + 1
    L["rootdir"] = WS
    L["args"] = keyed_args(plugin.args)
    L["tool"] = plugin.tool
    L["own"] = sorted(set(REC.own.values()))
    # The session: everything read outside a test file's windows.
    S = REC.S
    s_names = closure(S.imports | S.failed, REC.loads)
    s_windows = [S] + [REC.loads[m] for m in s_names if m in REC.loads]
    s_keys = window_keys(s_windows, s_names | S.failed, st)
    for k in ("py:", "sp:", "mp:", "dists:"):
        s_keys[k] = st.state(k)
    for name in st.dists()[1]:
        s_keys["d:" + name] = st.state("d:" + name)
    s_exec = set()
    for w in s_windows:
        s_exec |= w.exec
    unrec = None
    for w in s_windows:
        if w.unrec:
            unrec = w.unrec
            break
    if REC.thread_start is not None:
        extra = [t for t in REC.thread_start if t is not threading.main_thread() and t.is_alive()]
        if extra:
            unrec = unrec or "threads run beside the tests (%s)" % ", ".join(t.name for t in extra[:3])
    sess_changed = unrec is not None or L["session"].get("keys") != s_keys
    L["session"] = {"keys": s_keys, "exec": _code_map(s_exec), "unrecordable": unrec}
    # Collection layout for the next pre-scan.
    dirs = set()
    for f in plugin.collected_files:
        d = os.path.dirname(f) or "."
        while True:
            dirs.add(d)
            if d == ".":
                break
            d = os.path.dirname(d) or "."
    pats = list(plugin.config.getini("python_files"))
    L["collect"] = {"dirs": sorted(dirs), "patterns": pats}
    expect = None
    if OPTS.get("expect_report"):
        try:
            with open(OPTS["expect_report"]) as f:
                expect = json.load(f).get("tests", {})
        except (OSError, ValueError):
            expect = {}
    outcomes = plugin.outcomes()
    files = L["files"]
    # The new order: this session's files in collection order, merged with
    # the replayed ones at their positions.
    order = list(plugin.order) if plugin.order else []
    for f in plugin.collected_files:
        if f not in order:
            k = _sortkey(f)
            at = len(order)
            for i, g in enumerate(order):
                if _sortkey(g) > k:
                    at = i
                    break
            order.insert(at, f)
    if not plugin.order:
        order = list(plugin.collected_files)
    order = [f for f in order if os.path.exists(absp(f))]
    for f in list(files):
        if f not in order:
            files.pop(f)
    L["order"] = order
    blobs = os.path.join(OPTS["dir"], "blobs")
    os.makedirs(blobs, exist_ok=True)
    to_certify = []
    ran = [f for f in plugin.collected_files if f not in plugin.replay]
    for f in ran:
        w = REC.files.get(f) or Window("f", f)
        names = closure(w.imports | w.failed, REC.loads)
        windows = [w] + [REC.loads[m] for m in names if m in REC.loads and m not in s_names]
        keys = window_keys(windows, (names - s_names) | w.failed, st)
        keys["m:" + f] = keys.get("m:" + f) or st.state("m:" + f)
        if ("f:" + f) not in keys:
            keys.setdefault("m:" + f, st.state("m:" + f))
        ex = set()
        for x in windows:
            ex |= x.exec
        nodeids = plugin.items_by_file.get(f, [])
        items = []
        status = "passed"
        if plugin.collect_status.get(f) != "passed":
            status = "collect-%s" % plugin.collect_status.get(f)
        for nid in nodeids:
            t = plugin.results.get(nid)
            if t is None:
                status = "incomplete"
                continue
            o = outcome_of(t["reports"])
            if o.startswith("failed") or o.startswith("xpassed-strict"):
                status = "failed"
            kw, reps = compact(t["reports"])
            items.append({"nodeid": nid, "location": plugin.locs.get(nid), "keywords": kw, "reports": reps, "warnings": t["warnings"], "outcome": o})
        if plugin.collect_warnings.get(f):
            status = status if status != "passed" else "collect-warnings"
        unrec_f = w.unrec
        for x in windows:
            if x.unrec and unrec_f is None:
                unrec_f = x.unrec
        digest = hs(json.dumps(sorted(keys.items())))
        old = files.get(f) or {}
        rec = {
            "id": file_id(f),
            "keys": keys,
            "exec": _code_map(ex),
            "status": status,
            "unrecordable": unrec_f,
            "modules": sorted(names | s_names),
            "fixtures": sorted(plugin.fixtures.get(f, ())),
            "cleak": plugin.cleak.get(f) or {},
            "rleak": plugin.rleak.get(f) or {},
            "recorded": L["seq"],
            "audited": L["seq"] if f in plugin.audit else old.get("audited", 0),
            "certified": None,
            "quarantined": None,
            "digest": digest,
        }
        # Quarantine is sticky while the file's inputs are unchanged.
        if old.get("quarantined") and old.get("digest") == digest:
            rec["quarantined"] = old["quarantined"]
        if f in plugin.audit and old:
            prev = load_results(old["id"]) or {"items": []}
            was = {t["nodeid"]: norm_text(t["outcome"]) for t in prev["items"]}
            now = {t["nodeid"]: norm_text(t["outcome"]) for t in items}
            if was != now:
                rec["quarantined"] = "audit: its results differ from the recorded ones"
                summary["audit_mismatches"].append(f)
        if expect is not None and status == "passed":
            for t in items:
                e = expect.get(t["nodeid"])
                if e is None:
                    rec["status"] = "unconfirmed"
                    summary["not_recorded"][f] = "the critical path's report lacks %s" % t["nodeid"]
                    break
                if norm_text(e) != norm_text(t["outcome"]):
                    rec["quarantined"] = "recording changed behaviour (%s: %s here, %s on the critical path)" % (t["nodeid"], t["outcome"], e)
                    break
        if rec["quarantined"]:
            summary["quarantined"].append(f)
        if unrec_f:
            summary["unrecordable"][f] = unrec_f
        text = json.dumps([[t["reports"], t["warnings"]] for t in items])
        rec["quotes"] = sorted(p for p in set(rec["exec"]) | {k[2:] for k in keys if k.startswith("m:")} if re.search(r"(?<![\w.])" + re.escape(os.path.basename(p)), text))
        # Keep the old source of every module key (narrowing diffs it).
        for k, v in keys.items():
            if k.startswith("m:") and len(v) == 32:
                bp = os.path.join(blobs, v)
                if not os.path.exists(bp):
                    try:
                        with open(absp(k[2:]), "rb") as src:
                            data = src.read()
                        if h(data) == v:
                            with open(bp, "wb") as out:
                                out.write(data)
                    except OSError:
                        pass
        write_json(os.path.join(OPTS["dir"], "results", rec["id"] + ".json"), {"items": items, "deselected": plugin.deselected.get(f, 0)})
        files[f] = rec
        summary["recorded"] += 1
        if rec["status"] == "passed" and not rec["quarantined"] and not unrec_f:
            to_certify.append(f)
        elif old.get("certified") is False:
            rec["certified"] = False
    for k, v in s_keys.items():
        if k.startswith("m:") and len(v) == 32:
            bp = os.path.join(blobs, v)
            if not os.path.exists(bp):
                try:
                    with open(absp(k[2:]), "rb") as src:
                        data = src.read()
                    if h(data) == v:
                        with open(bp, "wb") as out:
                            out.write(data)
                except OSError:
                    pass
    summary["record_ms"] = int((time.monotonic() - t0) * 1000)
    t1 = time.monotonic()
    certify(plugin, to_certify, files, summary)
    summary["certify_ms"] = int((time.monotonic() - t1) * 1000)
    sc = dict(L.get("stat") or {})
    sc.update(st.new_sc)
    L["stat"] = {p: v for p, v in sc.items() if os.path.exists(p)}
    write_json(ledger_path(), L)
    summary["session_changed"] = sess_changed
    return summary


def certify(plugin, todo, files, summary):
    """A file is order-independent (it may replay while others run) when its
    results alone, in a fresh process, equal its results in the session."""
    if not todo:
        return
    from concurrent.futures import ThreadPoolExecutor

    known = list(plugin.collected_files) + [f for f in files if f not in plugin.collected_files]
    jobs = 1 if any(a.startswith("--basetemp") for a in plugin.args) else max(1, os.cpu_count() or 1)
    outdir = os.path.join(OPTS["dir"], "plugin", "alone")
    os.makedirs(outdir, exist_ok=True)

    def one(f):
        out = os.path.join(outdir, file_id(f) + ".json")
        opts = dict(OPTS)
        opts.update({"mode": "alone", "file": f, "ignore": known, "summary": out})
        op = out + ".opts"
        with open(op, "w") as fh:
            json.dump(opts, fh)
        try:
            os.unlink(out)
        except OSError:
            pass
        subprocess.run([sys.executable, _ME, op] + list(plugin.args), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd=START["cwd"])
        try:
            with open(out) as fh:
                return json.load(fh).get("tests")
        except (OSError, ValueError):
            return None

    with ThreadPoolExecutor(jobs) as ex:
        res = list(ex.map(one, todo))
    for f, alone in zip(todo, res):
        rec = files[f]
        session = {}
        prev = load_results(rec["id"]) or {"items": []}
        for t in prev["items"]:
            session[t["nodeid"]] = norm_text(t["outcome"])
        ok = alone is not None and {k: norm_text(v) for k, v in alone.items()} == session
        rec["certified"] = bool(ok)
        if ok:
            summary["certified"] += 1
        else:
            summary["uncertified"].append(f)


# ---------------------------------------------------------------------------
# main


def main():
    global OPTS, REC, WS
    with open(sys.argv[1]) as f:
        OPTS = json.load(f)
    args = sys.argv[2:]
    if args and args[0] == "--":
        args = args[1:]
    START["cwd"] = os.getcwd()
    # As `python -m pytest`: the working directory first on sys.path.
    sys.path[0] = os.getcwd()
    START["sys_path"] = [p for p in sys.path]
    START["meta"] = ",".join(type(x).__qualname__ for x in sys.meta_path)
    mode = OPTS["mode"]
    t0 = time.monotonic()
    if mode == "shadow":
        REC = Recorder()
        REC.install()
    import pytest

    sys.argv = [os.path.join(os.path.dirname(pytest.__file__), "__main__.py")] + args
    plugin = Quod(mode, args)
    rc = int(pytest.main(list(args), plugins=[plugin, _register(plugin)]))
    run_ms = int((time.monotonic() - t0) * 1000)
    summary = {
        "mode": mode,
        "exit": rc,
        "ran": len([f for f in plugin.collected_files if f not in plugin.replay]),
        "replayed": len(plugin.replayed_files),
        "replayed_tests": plugin.replayed_tests,
        "collected_only": sorted(plugin.collect_only),
        "narrowed": sorted(f for f in plugin.narrowed if f in plugin.replay),
        "run_files": sorted(f for f in plugin.collected_files if f not in plugin.replay),
        "reasons": plugin.reasons,
        "select_ms": plugin.select_ms,
        "pytest_ms": run_ms,
        "tripped": plugin.tripped,
        "unexpected": plugin.unexpected,
        "tests": plugin.outcomes(),
        "session_changed": plugin.session_changed[:20],
    }
    if mode == "critical" and plugin.tripped:
        # The guard: a file that ran changed shared state a replayed file can
        # observe. Replays cannot be trusted; run the whole suite.
        op = os.path.join(OPTS["dir"], "plugin", "plain-%d.json" % os.getpid())
        opts = dict(OPTS)
        opts.update({"mode": "plain", "summary": op + ".out"})
        with open(op, "w") as fh:
            json.dump(opts, fh)
        sys.stdout.flush()
        sys.stderr.flush()
        rc = subprocess.run([sys.executable, _ME, op] + list(args), cwd=START["cwd"]).returncode
        try:
            with open(op + ".out") as fh:
                full = json.load(fh)
            summary["tests"] = full.get("tests", {})
        except (OSError, ValueError):
            summary["tests"] = {}
        summary["exit"] = rc
        summary["ran"] = len(plugin.order) or summary["ran"]
        summary["replayed"] = 0
        summary["replayed_tests"] = 0
        summary["pytest_ms"] = int((time.monotonic() - t0) * 1000)
    if mode == "shadow":
        summary["shadow"] = record_shadow(plugin, rc)
        summary["shadow_ms"] = int((time.monotonic() - t0) * 1000)
        s = summary["shadow"]
        log(
            "shadow: recorded %d, certified %d, uncertified %d, quarantined %d, unrecordable %d, audit mismatches %d (record %d ms, certify %d ms)"
            % (
                s.get("recorded", 0),
                s.get("certified", 0),
                len(s.get("uncertified", [])),
                len(s.get("quarantined", [])),
                len(s.get("unrecordable", {})),
                len(s.get("audit_mismatches", [])),
                s.get("record_ms", 0),
                s.get("certify_ms", 0),
            )
        )
    if OPTS.get("summary"):
        write_json(OPTS["summary"], summary)
    if mode in ("critical", "shadow"):
        log(
            "%s: ran %d files, replayed %d (%d tests, %d narrowed), select %d ms%s"
            % (
                mode,
                summary["ran"],
                summary["replayed"],
                summary["replayed_tests"],
                len(summary["narrowed"]),
                summary["select_ms"],
                "; guard: " + plugin.tripped if plugin.tripped else "",
            )
        )
    sys.stdout.flush()
    sys.stderr.flush()
    return rc


if __name__ == "__main__":
    sys.modules.setdefault("quod_pytest", sys.modules["__main__"])
    sys.exit(main())
