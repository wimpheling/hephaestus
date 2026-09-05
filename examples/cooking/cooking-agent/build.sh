#!/bin/sh
set -eu
python3 -c 'import sqlite3, socket, tomllib; assert hasattr(socket, "AF_VSOCK")'
install -D -m 0755 cooking_agent.py /workspace/output/bin/cooking-agent
