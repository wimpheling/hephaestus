#!/usr/bin/env python3
"""Small, dependency-free HTTP collector for the Zot integration smoke test.

It intentionally records the binary CloudEvents headers and returns a status
selected at runtime. The production notification parser is tested elsewhere;
this fixture only proves Zot can deliver (and cannot synchronously block a
registry write when the sink is unavailable).
"""

from __future__ import annotations

import argparse
import base64
import http.server
import json
from pathlib import Path


class NotificationSink(http.server.ThreadingHTTPServer):
    output: Path
    status_file: Path


class Handler(http.server.BaseHTTPRequestHandler):
    server: NotificationSink

    def do_GET(self) -> None:  # noqa: N802 - HTTP method spelling is prescribed.
        if self.path == "/healthz":
            self.send_response(200)
            self.end_headers()
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self) -> None:  # noqa: N802 - HTTP method spelling is prescribed.
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        event = {
            "path": self.path,
            "headers": dict(self.headers.items()),
            "body_base64": base64.b64encode(body).decode("ascii"),
        }
        with self.server.output.open("a", encoding="utf-8") as output:
            output.write(json.dumps(event, sort_keys=True) + "\n")

        try:
            status = int(self.server.status_file.read_text(encoding="utf-8").strip())
        except (OSError, ValueError):
            status = 200
        self.send_response(status)
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--status-file", type=Path, required=True)
    arguments = parser.parse_args()

    server = NotificationSink(("127.0.0.1", arguments.port), Handler)
    server.output = arguments.output
    server.status_file = arguments.status_file
    server.serve_forever()


if __name__ == "__main__":
    main()
