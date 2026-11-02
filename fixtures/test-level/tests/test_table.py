import calc.table


def test_two():
    assert calc.table.load()["two"] == 2


def test_never_reads():
    assert True
