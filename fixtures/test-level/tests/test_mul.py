import pytest

from calc.mul import mul


@pytest.mark.parametrize("a,b,want", [(2, 3, 6), (4, 0, 0), (1, 7, 7)])
def test_mul(a, b, want):
    assert mul(a, b) == want
