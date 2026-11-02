import os

from calc import state

HERE = os.path.dirname(os.path.abspath(__file__))


def test_a_sets_state():
    state.value = 1


def test_b_needs_state():
    # Reads a file test_a does not, so a change to it re-runs this test
    # alone; it passes only when test_a ran first in the same process.
    with open(os.path.join(HERE, "..", "data", "b.txt")) as f:
        assert f.read().strip()
    assert state.value == 1
