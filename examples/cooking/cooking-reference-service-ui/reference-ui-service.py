#!/usr/local/bin/python3
"""Dependency-free long-lived managed release UI service fixture."""

from __future__ import annotations

import json
import os
from pathlib import Path
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlsplit


ADDRESS = ("127.0.0.1", 8080)
MAX_HEADER_BYTES = 8 * 1024
MAX_HEADER_COUNT = 32
REQUEST_TIMEOUT_SECONDS = 5
CSS_NAME = "heph-ui-kit-v1.0.0.css"
JS_NAME = "heph-ui-kit-v1.0.0.js"


class ReferenceHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "reference-ui-service"
    sys_version = ""

    def setup(self) -> None:
        super().setup()
        self.connection.settimeout(REQUEST_TIMEOUT_SECONDS)

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_GET(self) -> None:
        if len(self.headers) > MAX_HEADER_COUNT or sum(
            len(name) + len(value) for name, value in self.headers.items()
        ) > MAX_HEADER_BYTES:
            self._error(HTTPStatus.REQUEST_HEADER_FIELDS_TOO_LARGE)
            return

        path = urlsplit(self.path).path
        if path in {"/readyz", "/healthz"}:
            self._send(HTTPStatus.OK, b"ready\n" if path == "/readyz" else b"healthy\n", "text/plain")
        elif path in {"/reference", "/reference/", "/reference/index.html"}:
            self._send_file("index.html", "text/html; charset=utf-8")
        elif path == f"/reference/{CSS_NAME}":
            self._send_file(CSS_NAME, "text/css; charset=utf-8")
        elif path == f"/reference/{JS_NAME}":
            self._send_file(JS_NAME, "text/javascript; charset=utf-8")
        elif path == "/reference/identity":
            body = json.dumps(self.server.startup_identity, sort_keys=True).encode() + b"\n"
            self._send(HTTPStatus.OK, body, "application/json")
        else:
            self._error(HTTPStatus.NOT_FOUND)

    def do_HEAD(self) -> None:
        self._error(HTTPStatus.METHOD_NOT_ALLOWED)

    def do_POST(self) -> None:
        self._error(HTTPStatus.METHOD_NOT_ALLOWED)

    def _error(self, status: HTTPStatus) -> None:
        body = f"{status.value} {status.phrase}\n".encode("ascii")
        self._send(status, body, "text/plain; charset=utf-8")

    def _send_file(self, name: str, media_type: str) -> None:
        root = Path(__file__).resolve().parent
        self._send(HTTPStatus.OK, (root / name).read_bytes(), media_type)

    def _send(self, status: HTTPStatus, body: bytes, media_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", media_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)


class ReferenceServer(HTTPServer):
    allow_reuse_address = False

    def __init__(self) -> None:
        super().__init__(ADDRESS, ReferenceHandler)
        startup = f"{os.getpid()}-{time.time_ns()}"
        self.startup_identity = {"pid": os.getpid(), "startup_id": startup}


def main() -> None:
    server = ReferenceServer()
    print("managed-reference-ui ready", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
