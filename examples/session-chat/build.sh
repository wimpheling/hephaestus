#!/bin/sh
set -eu

python3 -m py_compile agent.py git_adapter.py protocol.py
output_root=${HEPH_OUTPUT_ROOT:-/workspace/output}
install -d -m 0755 "$output_root/bin/session-chat"
install -m 0755 agent.py "$output_root/bin/session-chat/session-chat-agent"
install -m 0644 git_adapter.py protocol.py "$output_root/bin/session-chat/"
