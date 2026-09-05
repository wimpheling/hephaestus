#!/usr/bin/env python3
"""External deterministic relay. No real Telegram transport is enabled."""
import hashlib
import hmac
import json
import sqlite3
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import argparse


class Relay:
    def __init__(self, database, credential):
        self.db = sqlite3.connect(database)
        self.db.execute('PRAGMA journal_mode=WAL')
        self.db.execute('PRAGMA synchronous=FULL')
        self.db.execute('CREATE TABLE IF NOT EXISTS deliveries(key TEXT PRIMARY KEY, request TEXT NOT NULL, outcome TEXT NOT NULL)')
        self.credential = credential

    def deliver(self, authorization, raw):
        if not hmac.compare_digest(authorization.encode(), b'Bearer ' + self.credential):
            return 401, {'error': 'unauthorized'}
        if len(raw) > 8192:
            return 400, {'error': 'invalid request'}
        try:
            body = json.loads(raw)
            if (not isinstance(body, dict) or set(body) != {'idempotency_key', 'user_id', 'text'}
                    or body['user_id'] not in ('alice', 'bob')
                    or not isinstance(body['text'], str) or not 0 < len(body['text'].encode()) <= 2048
                    or not isinstance(body['idempotency_key'], str) or not 0 < len(body['idempotency_key']) <= 128
                    or not all(32 <= ord(c) < 127 for c in body['idempotency_key'])):
                raise ValueError('invalid request')
        except (ValueError, TypeError):
            return 400, {'error': 'invalid request'}
        request = json.dumps(body, sort_keys=True, separators=(',', ':'))
        key = body['idempotency_key']
        with self.db:
            self.db.execute('BEGIN IMMEDIATE')
            existing = self.db.execute('SELECT request,outcome FROM deliveries WHERE key=?', (key,)).fetchone()
            if existing:
                if existing[0] != request:
                    return 409, {'error': 'idempotency conflict'}
                return 200, json.loads(existing[1])
            # Deterministic external transport outcome: no network or provider token.
            outcome = {'status': 'delivered', 'message_id': 'fake-' + hashlib.sha256(key.encode()).hexdigest()[:24]}
            self.db.execute('INSERT INTO deliveries VALUES(?,?,?)', (key, request, json.dumps(outcome)))
        return 200, outcome


def serve(relay, port):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            self.connection.settimeout(10)
            if self.path != '/v1/messages':
                status, body = 404, {'error': 'not found'}
            else:
                try:
                    lengths = self.headers.get_all('content-length', [])
                    if len(lengths) != 1 or self.headers.get('transfer-encoding'):
                        raise ValueError('invalid framing')
                    size = int(lengths[0])
                    if not 0 < size <= 8192:
                        raise ValueError('invalid size')
                    auth = self.headers.get_all('authorization', [])
                    status, body = relay.deliver(auth[0] if len(auth) == 1 else '', self.rfile.read(size))
                except (AssertionError, ValueError, TimeoutError):
                    status, body = 400, {'error': 'invalid request'}
            raw = json.dumps(body).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
    HTTPServer(('127.0.0.1', port), Handler).serve_forever()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--database', required=True)
    parser.add_argument('--credential-file', required=True)
    parser.add_argument('--port', type=int, default=8091)
    args = parser.parse_args()
    credential = Path(args.credential_file).read_bytes().strip()
    if not 16 <= len(credential) <= 256:
        raise SystemExit('invalid credential file')
    serve(Relay(args.database, credential), args.port)
