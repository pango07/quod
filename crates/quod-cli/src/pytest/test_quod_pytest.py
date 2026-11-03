# Unit tests of quod_pytest's pure functions (narrowing, the version rule).
# Run in CI (pango07/ci-bench bench-pytest.yml runs them before every bench):
#     python -m pytest crates/quod-cli/src/pytest/test_quod_pytest.py
import importlib.util
import os

_spec = importlib.util.spec_from_file_location("quod_pytest_under_test", os.path.join(os.path.dirname(__file__), "quod_pytest.py"))
q = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(q)


OLD = '''from faker.factory import Factory

VERSION = "40.43.0"

def f(x):
    """Doc."""
    y = x + 1
    return y

def g():
    return 2
'''


def test_version_only_bump():
    new = OLD.replace("40.43.0", "40.44.0")
    names, values = q.version_only(OLD, new)
    assert names == {"VERSION"}
    assert values == {"40.43.0", "40.44.0"}


def test_version_only_rejects_other_changes():
    new = OLD.replace("40.43.0", "40.44.0").replace("return 2", "return 3")
    assert q.version_only(OLD, new) is None
    # a non-version constant
    assert q.version_only("NAME = 'a'\n", "NAME = 'b'\n") is None
    # nothing changed
    assert q.version_only(OLD, OLD) is None


def test_mentions_text():
    assert q.mentions_text("from faker import VERSION\n", {"VERSION"})
    assert q.mentions_text("print(faker.VERSION)\n", {"VERSION"})
    assert not q.mentions_text("VERSIONS = 1\nMY_VERSION = 2\n", {"VERSION"})
    # the definition itself does not count, another use does
    assert not q.mentions_text('VERSION = "1"\n', {"VERSION"}, skip_defs=True)
    assert q.mentions_text('VERSION = "1"\nx = VERSION\n', {"VERSION"}, skip_defs=True)


def test_narrowing_inside_unexecuted_body():
    new = OLD.replace("y = x + 1", "y = x + 2")
    assert q.changed_functions(OLD, new) == {(5, "f")}
    assert q.narrowable(OLD, new, {(10, "g")})
    assert not q.narrowable(OLD, new, {(5, "f")})


def test_narrowing_refuses_docstring_signature_and_module_level():
    assert q.changed_functions(OLD, OLD.replace('"""Doc."""', '"""Other."""')) is None
    assert q.changed_functions(OLD, OLD.replace("def f(x):", "def f(x, z=1):")) is None
    assert q.changed_functions(OLD, OLD + "\nZ = 1\n") is None
    # a statement dedented out of the body
    assert q.changed_functions(OLD, OLD.replace("    return 2\n", "    return 2\nprint(1)\n")) is None
