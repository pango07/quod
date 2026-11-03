# quod pytest's launcher: imports the plugin as a module (its bytecode is
# cached in __pycache__; a script would be compiled on every run).
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import quod_pytest  # noqa: E402

sys.exit(quod_pytest.main())
