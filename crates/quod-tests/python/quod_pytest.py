"""Quod's pytest plugin: test-level replay (docs/test-level.md).

Quod injects this module into a pytest step (PYTHONPATH and PYTEST_PLUGINS)
when it runs the step at test level. It does three things, and nothing at
all unless Quod set QUOD_TL_DIR:

1. Marks windows. Before and after each test's setup/call/teardown, each
   test module's collection, each module- or session-scoped fixture's setup
   and teardown, and each module's first import, it logs a numbered marker
   to its event file and stats a marker path under the workspace root. The
   observer logs that lookup in order with every file access the process
   and its children make, so Quod can attribute each read to the window it
   happened in. It also records which modules each window imported (cached
   or not), so a test depends on what its imports read when they were
   first loaded, whichever window loaded them.
2. Records each executed test's reports (setup, call, teardown, as pytest
   serializes them for xdist) and warnings.
3. Replays: the tests Quod's plan names are taken out of the run (as -k
   would) and their recorded reports are fed to pytest's reporting hooks,
   so the summary counts and the exit code are what a full run gives.

Everything here is conservative: a condition it cannot handle marks the
run tainted, and Quod then records nothing at test level.
"""

import builtins
import json
import os
import sys
import threading
import time

import pytest

PLUGIN_VERSION = 1

_DIR = os.environ.get("QUOD_TL_DIR")
_TOKEN = os.environ.get("QUOD_TL_TOKEN", "")
_ROOT = os.environ.get("QUOD_TL_ROOT", "/workspace")

# Options and plugins under which a run's outcome depends on which tests
# run or in what order beyond what replay preserves: test-level replay is
# off for such a run (Quod records nothing at test level).
_UNSAFE_PLUGINS = (
    "randomly",
    "random_order",
    "rerunfailures",
    "forked",
    "flaky",
    "testmon",
    "picked",
)


class _State:
    """One process's test-level state."""

    def __init__(self, role, workerid):
        self.role = role  # "main", "worker" or "master"
        self.workerid = workerid
        self.n = 0
        self.events = []  # [n, "b"|"e", kind, id]
        self.stack = []  # [(n, kind, id)]
        self.edges = {}  # begin n (0: no window) -> set of module names
        self.offthread = set()  # begin n of windows an import ran off-thread in
        self.collected = []
        self.tests = {}  # nodeid -> {"module", "reports", "warnings"}
        self.replayed = []
        self.refused = []
        self.taint = None
        self.done = False
        self.exitstatus = None
        self.plan = {}
        self.pending = []  # (index, item, record) taken out, to report
        self.replays_seen = 0
        self.index = {}
        self.main_thread = threading.main_thread()

    # Markers -----------------------------------------------------------

    def _marker(self, n):
        try:
            os.stat(os.path.join(_ROOT, ".quod-tl-%s-%d-%d" % (_TOKEN, os.getpid(), n)))
        except OSError:
            pass

    def begin(self, kind, ident):
        if threading.current_thread() is not self.main_thread:
            self.offthread.add(self.current())
            return None
        self.n += 1
        n = self.n
        self.events.append([n, "b", kind, ident])
        self._marker(n)
        self.stack.append((n, kind, ident))
        return n

    def end(self, token):
        if token is None:
            return
        # Close exactly the window opened (and any left open inside it,
        # which an exception unwinding through a hook can leave).
        while self.stack:
            n0, kind, ident = self.stack.pop()
            self.n += 1
            self.events.append([self.n, "e", kind, ident])
            self._marker(self.n)
            if n0 == token:
                return
        self.taint = self.taint or "window stack unbalanced"

    def current(self):
        return self.stack[-1][0] if self.stack else 0

    def imported(self, names):
        if threading.current_thread() is not self.main_thread:
            # An import in another thread: its window is not this thread's.
            self.offthread.add(self.current())
            return
        self.edges.setdefault(self.current(), set()).update(names)

    # Output ------------------------------------------------------------

    def dump(self):
        if not _DIR:
            return
        out = {
            "v": PLUGIN_VERSION,
            "pid": os.getpid(),
            "role": self.role,
            "workerid": self.workerid,
            "events": self.events,
            "edges": {str(k): sorted(v) for k, v in self.edges.items()},
            "offthread": sorted(self.offthread),
            "collected": self.collected,
            "tests": self.tests,
            "replayed": self.replayed,
            "refused": self.refused,
            "taint": self.taint,
            "done": self.done,
            "exitstatus": self.exitstatus,
            "python": list(sys.version_info[:3]),
            "imports_tracked": _IMPORTS_TRACKED,
        }
        path = os.path.join(_DIR, "out", "%d.json" % os.getpid())
        tmp = path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(out, f, sort_keys=True)
        os.replace(tmp, path)


_STATE = None
_IMPORTS_TRACKED = False


# Import tracking ---------------------------------------------------------


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


def _install_import_tracking():
    """Wraps the import system's module execution (each first load gets a
    window) and both entry points of an import statement (each import, cached
    or not, is an edge from the current window). False if this Python's
    import system is not the one these wrappers expect."""
    import importlib
    import importlib._bootstrap as bootstrap

    if not hasattr(bootstrap, "_load_unlocked") or not hasattr(bootstrap, "_gcd_import"):
        return False
    orig_load = bootstrap._load_unlocked
    orig_gcd = bootstrap._gcd_import
    orig_import = builtins.__import__
    resolve = getattr(bootstrap, "_resolve_name", None)
    calc_package = getattr(bootstrap, "_calc___package__", None)
    if resolve is None or calc_package is None:
        return False

    def load_unlocked(spec):
        st = _STATE
        if st is None:
            return orig_load(spec)
        name = spec.name
        # A first load inside a window is that window's dependency too.
        st.imported({name})
        tok = st.begin("l", name)
        try:
            return orig_load(spec)
        finally:
            st.end(tok)

    def gcd_import(name, package=None, level=0):
        mod = orig_gcd(name, package, level)
        st = _STATE
        if st is not None:
            try:
                absname = resolve(name, package, level) if level > 0 else name
                st.imported(_deps_of(absname, ()))
            except Exception:
                st.taint = st.taint or "import edge not resolved"
        return mod

    def quod_import(name, globals=None, locals=None, fromlist=(), level=0):
        mod = orig_import(name, globals, locals, fromlist, level)
        st = _STATE
        if st is not None:
            try:
                if level > 0:
                    absname = resolve(name, calc_package(globals), level) if name else calc_package(globals)
                else:
                    absname = name
                st.imported(_deps_of(absname, fromlist))
            except Exception:
                st.taint = st.taint or "import edge not resolved"
        return mod

    bootstrap._load_unlocked = load_unlocked
    bootstrap._gcd_import = gcd_import
    builtins.__import__ = quod_import
    # importlib.import_module looks _gcd_import up on the module each call.
    del importlib
    return True


# Activation --------------------------------------------------------------


def _plan():
    try:
        with open(os.path.join(_DIR, "plan.json")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def _unsafe_reason(config):
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
    for p in _UNSAFE_PLUGINS:
        if config.pluginmanager.hasplugin(p):
            if p == "randomly" and getattr(opt, "randomly_seed", None) == "last":
                pass
            return "plugin %s changes order or reruns tests" % p
    return None


def pytest_configure(config):
    global _STATE, _IMPORTS_TRACKED
    if not _DIR or _STATE is not None:
        # Not under Quod, or an in-process nested session (pytester): its
        # reads are the enclosing test's.
        return
    workerinput = getattr(config, "workerinput", None)
    if workerinput is not None:
        if workerinput.get("quod_tl") != _TOKEN:
            return
        role, workerid = "worker", workerinput.get("workerid")
    else:
        # The step's first pytest session owns the run; any other (a second
        # pytest in the script, one a test runs) is left alone, and its
        # reads, outside every window, are the whole session's.
        try:
            fd = os.open(os.path.join(_DIR, "owner"), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        except OSError:
            return
        os.write(fd, str(os.getpid()).encode())
        os.close(fd)
        numprocesses = getattr(config.option, "numprocesses", None)
        role = "master" if numprocesses not in (None, 0, "0") and config.pluginmanager.hasplugin("xdist") else "main"
        workerid = None
    st = _State(role, workerid)
    plan = _plan()
    if plan is None or plan.get("token") != _TOKEN:
        st.taint = "no plan"
    else:
        st.plan = plan.get("replay", {})
    st.taint = st.taint or _unsafe_reason(config)
    _STATE = st
    if role != "master":
        try:
            _IMPORTS_TRACKED = _install_import_tracking()
        except Exception:
            _IMPORTS_TRACKED = False
        if not _IMPORTS_TRACKED:
            st.taint = st.taint or "import tracking unavailable"
        _patch_fixture_finish()
    hooks = _Hooks(st, config)
    if role == "master" and config.pluginmanager.hasplugin("xdist"):
        config.pluginmanager.register(_XdistMaster(hooks), "quod-tl-xdist")
    config.pluginmanager.register(hooks, "quod-tl")
    import atexit

    atexit.register(st.dump)


class _XdistMaster:
    """Under xdist the master reports the replayed tests, once: every worker
    takes the same tests out of its collection (so the collections agree)
    and writes which, with their locations, before it sends its collection;
    the master reports them when the first collection arrives."""

    def __init__(self, hooks):
        self.hooks = hooks
        self.reported = False

    def pytest_configure_node(self, node):
        node.workerinput["quod_tl"] = _TOKEN

    def pytest_xdist_node_collection_finished(self, node, ids):
        st = self.hooks.st
        if self.reported or st.taint:
            return
        self.reported = True
        wid = node.gateway.id
        try:
            with open(os.path.join(_DIR, "out", "removed-%s.json" % wid)) as f:
                removed = json.load(f)
        except (OSError, ValueError):
            st.taint = "worker %s wrote no list of replayed tests" % wid
            return
        for r in removed:
            rec = st.plan.get(r["nodeid"])
            if rec is None:
                st.taint = "a worker took out a test the plan does not replay"
                return
            self.hooks._report(r["nodeid"], tuple(r["location"]), rec, self.hooks.config.hook)


def _module_of(node):
    if node is None:
        return None
    if isinstance(node, pytest.Module):
        return node
    return node.getparent(pytest.Module)


def _patch_fixture_finish():
    """A module- or session-scoped fixture's teardown runs inside whichever
    test happens to be last to use it; its reads belong to its scope."""
    from _pytest import fixtures

    orig = fixtures.FixtureDef.finish
    if getattr(orig, "_quod", False):
        return

    def finish(self, request, *a, **kw):
        st = _STATE
        tok = None
        if st is not None:
            tok = _fixture_window(st, self.scope, request)
        try:
            return orig(self, request, *a, **kw)
        finally:
            if st is not None:
                st.end(tok)

    finish._quod = True
    fixtures.FixtureDef.finish = finish


def _fixture_window(st, scope, request):
    scope = getattr(scope, "value", scope)
    if scope in ("session", "package"):
        return st.begin("s", "")
    if scope in ("module", "class"):
        mod = _module_of(getattr(request, "node", None))
        if mod is None:
            return st.begin("s", "")
        return st.begin("m", mod.nodeid)
    return None


def _strip(data):
    """A serialized report without what differs between faithful runs."""
    data = dict(data)
    for k in ("duration", "start", "stop"):
        data.pop(k, None)
    return data


def _warning_category(qualname):
    mod, _, name = qualname.rpartition(".")
    if not mod:
        return getattr(builtins, name, None)
    m = sys.modules.get(mod)
    if m is None:
        return None
    obj = m
    for part in name.split("."):
        obj = getattr(obj, part, None)
    return obj if isinstance(obj, type) and issubclass(obj, Warning) else None


class _Hooks:
    def __init__(self, st, config):
        self.st = st
        self.config = config

    # Collection --------------------------------------------------------

    @pytest.hookimpl(hookwrapper=True)
    def pytest_make_collect_report(self, collector):
        st = self.st
        mod = _module_of(collector)
        tok = st.begin("c", mod.nodeid) if (mod is not None and st.role != "master") else None
        try:
            outcome = yield
        finally:
            st.end(tok)
        return outcome

    @pytest.hookimpl(trylast=True)
    def pytest_collection_modifyitems(self, session, config, items):
        st = self.st
        if st.role == "master":
            return
        st.collected = [it.nodeid for it in items]
        if st.taint:
            return
        keep, out = [], []
        for i, it in enumerate(items):
            rec = st.plan.get(it.nodeid)
            if rec is not None and self._replayable(rec):
                out.append((i, it, rec))
            else:
                if rec is not None:
                    st.refused.append(it.nodeid)
                keep.append(it)
        st.index = {it.nodeid: i for i, it in enumerate(items)}
        items[:] = keep
        if st.role == "worker":
            # The master reports them (`_XdistMaster`).
            path = os.path.join(_DIR, "out", "removed-%s.json" % st.workerid)
            with open(path + ".tmp", "w") as f:
                json.dump([{"nodeid": it.nodeid, "location": list(it.location)} for _, it, _ in out], f)
            os.replace(path + ".tmp", path)
            out = []
        st.pending = out

    def _replayable(self, rec):
        for w in rec.get("warnings", []):
            if _warning_category(w.get("category", "")) is None:
                return False
        return True

    # Running -----------------------------------------------------------

    def _flush(self, before):
        """Reports the replayed tests that come before original index
        `before` (all of them for None). Under xdist only worker gw0
        reports them, once."""
        st = self.st
        while st.pending and (before is None or st.pending[0][0] < before):
            _, item, rec = st.pending.pop(0)
            self._report(item.nodeid, item.location, rec, item.ihook)

    def _report(self, nodeid, location, rec, ihook):
        st = self.st
        config = self.config
        st.replayed.append(nodeid)
        ihook.pytest_runtest_logstart(nodeid=nodeid, location=location)
        now = time.time()
        for data in rec["reports"]:
            data = dict(data)
            data.setdefault("duration", 0.0)
            data.setdefault("start", now)
            data.setdefault("stop", now)
            # A skip's (path, line, reason) came back from JSON as a list.
            if isinstance(data.get("longrepr"), list):
                data["longrepr"] = tuple(data["longrepr"])
            rep = config.hook.pytest_report_from_serializable(config=config, data=data)
            rep.quod_replayed = True
            ihook.pytest_runtest_logreport(report=rep)
        for w in rec.get("warnings", []):
            import warnings

            cat = _warning_category(w["category"])
            msg = warnings.WarningMessage(cat(w["message"]), cat, w["filename"], w["lineno"])
            config.hook.pytest_warning_recorded.call_historic(
                kwargs=dict(warning_message=msg, when="runtest", nodeid=nodeid, location=None)
            )
        ihook.pytest_runtest_logfinish(nodeid=nodeid, location=location)

    @pytest.hookimpl(hookwrapper=True)
    def pytest_runtestloop(self, session):
        st = self.st
        if st.role == "main" and st.pending:
            # Counted as collected, so a run whose every test replays is not
            # "no tests ran".
            session.testscollected += len(st.pending)
        outcome = yield
        if st.role == "main":
            self._flush(None)
        return outcome

    @pytest.hookimpl(hookwrapper=True)
    def pytest_runtest_protocol(self, item, nextitem):
        st = self.st
        if st.role == "master":
            outcome = yield
            return outcome
        self._flush(st.index.get(item.nodeid))
        mod = _module_of(item)
        st.tests[item.nodeid] = {
            "module": mod.nodeid if mod is not None else None,
            "reports": [],
            "warnings": [],
            "duration": 0.0,
        }
        tok = st.begin("t", item.nodeid)
        try:
            outcome = yield
        finally:
            st.end(tok)
        return outcome

    @pytest.hookimpl(hookwrapper=True)
    def pytest_fixture_setup(self, fixturedef, request):
        st = self.st
        tok = _fixture_window(st, fixturedef.scope, request) if st.role != "master" else None
        try:
            outcome = yield
        finally:
            st.end(tok)
        return outcome

    def pytest_runtest_logreport(self, report):
        st = self.st
        if getattr(report, "quod_replayed", False):
            st.replays_seen += 1
            return
        if st.role == "master":
            return
        t = st.tests.get(report.nodeid)
        if t is None:
            return
        data = self.config.hook.pytest_report_to_serializable(config=self.config, report=report)
        t["reports"].append(_strip(data))
        t["duration"] += getattr(report, "duration", 0.0) or 0.0

    def pytest_warning_recorded(self, warning_message, when, nodeid, location):
        st = self.st
        if st.role == "master" or when != "runtest" or nodeid in st.replayed:
            return
        t = st.tests.get(nodeid)
        if t is None:
            st.taint = st.taint or "a warning outside a recorded test's window"
            return
        cat = warning_message.category
        t["warnings"].append(
            {
                "category": "%s.%s" % (cat.__module__, cat.__qualname__)
                if cat.__module__ != "builtins"
                else cat.__qualname__,
                "message": str(warning_message.message),
                "filename": warning_message.filename,
                "lineno": warning_message.lineno,
            }
        )

    def pytest_report_teststatus(self, report, config):
        if getattr(report, "quod_replayed", False) and report.when == "call" and report.passed:
            return "passed", ".", ("PASSED (replayed)", {"green": True})
        return None

    def pytest_terminal_summary(self, terminalreporter):
        st = self.st
        n = len(st.replayed)
        if n:
            terminalreporter.write_line("quod: %d tests replayed (their inputs did not change)" % n)

    @pytest.hookimpl(tryfirst=True)
    def pytest_sessionfinish(self, session, exitstatus):
        st = self.st
        if st.replays_seen and int(session.exitstatus) == int(pytest.ExitCode.NO_TESTS_COLLECTED):
            # Every test replayed (xdist's master never saw one run).
            session.exitstatus = pytest.ExitCode.OK
        st.done = True
        st.exitstatus = int(session.exitstatus)
        st.dump()
