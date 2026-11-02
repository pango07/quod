import json
import os

_HERE = os.path.dirname(os.path.abspath(__file__))


def load():
    # Read when called, not at import: the test that calls it reads it.
    with open(os.path.join(_HERE, "..", "..", "data", "table.json")) as f:
        return json.load(f)
