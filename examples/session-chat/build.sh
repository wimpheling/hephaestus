#!/bin/sh
set -eu

python3 - <<'PY'
from pathlib import Path

for source in ("agent.py", "git_adapter.py", "protocol.py"):
    path = Path(source)
    compile(path.read_text(encoding="utf-8"), str(path), "exec", dont_inherit=True)
PY
output_root=${HEPH_OUTPUT_ROOT:-/workspace/output}
install -d -m 0755 "$output_root/bin/session-chat"
install -m 0755 agent.py "$output_root/bin/session-chat/session-chat-agent"
install -m 0644 git_adapter.py protocol.py "$output_root/bin/session-chat/"

test -f ui/dist/session-chat-ui.js
install -d -m 0755 "$output_root/dist"
install -m 0644 ui/index.html "$output_root/dist/index.html"
install -m 0644 ui/dist/session-chat-ui.js "$output_root/dist/session-chat-ui.js"
install -m 0644 ui/dist/session-chat-ui.css "$output_root/dist/session-chat-ui.css"
