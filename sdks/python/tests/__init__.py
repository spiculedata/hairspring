"""Tests for the Hairspring Python SDK. Importing this package puts `src` on the import path, so the tests run with

    python3 -m unittest discover -s sdks/python/tests -t sdks/python

from the repository root, with no installation step."""

import os
import sys

SDK_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC_DIR = os.path.join(SDK_DIR, "src")

if SRC_DIR not in sys.path:
    sys.path.insert(0, SRC_DIR)

# The SDK starts when it is imported and would write its report to this process's stderr at exit.
os.environ["HAIRSPRING_EVENTS"] = "off"
