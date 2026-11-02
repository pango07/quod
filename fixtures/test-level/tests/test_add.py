from calc.add import add


def test_add():
    assert add(2, 3) == 5


def test_add_base(base):
    assert add(base, 1) == 11
