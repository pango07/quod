import pytest


@pytest.fixture(scope="session")
def base():
    return 10
