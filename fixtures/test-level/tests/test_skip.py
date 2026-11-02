import warnings

import pytest


@pytest.mark.skip(reason="always skipped")
def test_skipped():
    pass


@pytest.mark.xfail(reason="known")
def test_xfail():
    assert False


def test_warns():
    warnings.warn("careful", UserWarning)
