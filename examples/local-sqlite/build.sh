#!/bin/sh
set -eu
python3 - <<'PY'
from pathlib import Path
import sqlite3
import tomllib
assert sqlite3.sqlite_version_info >= (3, 24, 0)
compile(Path('app.py').read_text(encoding='utf-8'), 'app.py', 'exec', dont_inherit=True)
PY
output_root=${HEPH_OUTPUT_ROOT:-/workspace/output}
install -D -m 0755 app.py "$output_root/bin/local-sqlite"
