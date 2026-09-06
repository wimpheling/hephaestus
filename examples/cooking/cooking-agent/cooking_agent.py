#!/usr/local/bin/python3
"""Released cooking loop. Provider credentials never enter this process."""
import argparse
import html
import json
from pathlib import Path
import socket
import sqlite3
import struct
import subprocess


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def bounded_text(value, limit):
    if not isinstance(value, str) or not value.strip() or len(value.encode()) > limit:
        raise ValueError("invalid bounded text")
    if any(ord(c) < 32 and c != "\n" for c in value):
        raise ValueError("invalid control character")
    return value.strip()


def validate_event(event):
    if not isinstance(event, dict) or set(event) != {"provider_update_id", "user_id", "command", "text"}:
        raise ValueError("invalid event fields")
    if type(event["provider_update_id"]) is not int or not 0 <= event["provider_update_id"] < 2**63:
        raise ValueError("invalid update ID")
    if event["user_id"] not in ("alice", "bob") or event["command"] != "recipe":
        raise ValueError("invalid identity or command")
    bounded_text(event["text"], 2048)


def connect(path):
    db = sqlite3.connect(path)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=FULL")
    db.executescript("""
        CREATE TABLE IF NOT EXISTS schema_meta(version INTEGER NOT NULL);
        INSERT INTO schema_meta SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_meta);
        CREATE TABLE IF NOT EXISTS users(user_id TEXT PRIMARY KEY, preferences TEXT NOT NULL);
        INSERT OR IGNORE INTO users VALUES('alice',''),('bob','');
        CREATE TABLE IF NOT EXISTS processed_updates(update_id INTEGER PRIMARY KEY,
            disposition TEXT NOT NULL, result_identity TEXT NOT NULL, event TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS recipes(recipe_id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
            request_summary TEXT NOT NULL, rendered_content TEXT, publication_outcome TEXT NOT NULL,
            model_response TEXT, relay_outcome TEXT, context TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS conversation_summaries(user_id TEXT PRIMARY KEY, summary TEXT NOT NULL);
    """)
    return db


def migrate(db, fail=False):
    with db:
        db.execute("BEGIN IMMEDIATE")
        version = db.execute("SELECT version FROM schema_meta").fetchone()[0]
        if version == 2:
            return
        if version != 1:
            raise ValueError("unsupported state version")
        db.execute("ALTER TABLE recipes ADD COLUMN recipe_summary TEXT NOT NULL DEFAULT ''")
        db.execute("UPDATE recipes SET recipe_summary=substr(request_summary,1,256)")
        db.execute("CREATE INDEX recipes_summary ON recipes(recipe_summary)")
        db.execute("UPDATE schema_meta SET version=2")
        if fail:
            raise ValueError("deliberate migration rollback")


def read_exact(stream, length):
    data = bytearray()
    while len(data) < length:
        part = stream.recv(length - len(data))
        if not part:
            raise ValueError("truncated broker frame")
        data.extend(part)
    return bytes(data)


class Broker:
    def __init__(self, parameters, control=Path('/run/hephaestus')):
        self.parameters = parameters
        self.context = json.loads((control / 'context.json').read_bytes())

    def __call__(self, slot, body):
        model = slot == 'model'
        destination = 'api.model.example' if model else 'relay.cooking.example'
        rule = self.parameters['model_rule_id' if model else 'relay_rule_id']
        placeholder = 'heph-placeholder:v1:' + rule
        request = {'rule_id': rule, 'method': 'post',
                   'path_and_query': '/v1/recipes' if model else '/v1/messages',
                   'headers': [{'name': 'authorization', 'value': 'Bearer ' + placeholder},
                               {'name': 'content-type', 'value': 'application/json'}],
                   'body': list(encoded(body))}
        credential = Path('/run/hephaestus-secrets/.runtime-credential').read_bytes()
        if len(credential) != 32:
            raise ValueError('invalid broker credential')
        wire = {'credential': list(credential), 'run_id': self.context['run_id'], 'slot': slot,
                'destination': destination, 'operation': 'https_v1', 'body': list(encoded(request))}
        payload = encoded(wire)
        with socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) as stream:
            stream.settimeout(15)
            stream.connect((2, 19001))
            stream.sendall(struct.pack('!I', len(payload)) + payload)
            length = struct.unpack('!I', read_exact(stream, 4))[0]
            if not 0 < length <= 1048576:
                raise ValueError('invalid broker frame')
            response = json.loads(read_exact(stream, length))
        if response['status'] != 'succeeded' or len(response['body']) > 16384:
            raise ValueError('broker operation unsuccessful')
        return json.loads(bytes(response['body']))


def validate_recipe(recipe):
    if not isinstance(recipe, dict) or set(recipe) != {'title', 'summary', 'ingredients', 'steps'}:
        raise ValueError('invalid model fields')
    bounded_text(recipe['title'], 128)
    bounded_text(recipe['summary'], 512)
    for key in ('ingredients', 'steps'):
        if not isinstance(recipe[key], list) or not 1 <= len(recipe[key]) <= 24:
            raise ValueError('invalid recipe list')
        for item in recipe[key]:
            bounded_text(item, 256)


def safe_markdown(value):
    # Model output is plain text, never executable Hugo shortcodes or raw HTML.
    value = html.escape(value, quote=False).replace('{', '&#123;').replace('}', '&#125;')
    for char in '\\`*_[]!#':
        value = value.replace(char, '\\' + char)
    return value.replace('\n', ' ')


def render(recipe, version):
    title = json.dumps(recipe['title'], ensure_ascii=False)
    output = f'+++\ntitle = {title}\ndraft = false\n+++\n\n'
    if version == 2:
        output += safe_markdown(recipe['summary']) + '\n\n'
    output += '## Ingredients\n\n' + ''.join('- ' + safe_markdown(x) + '\n' for x in recipe['ingredients'])
    output += '\n## Method\n\n' + ''.join(f'{i}. {safe_markdown(x)}\n' for i, x in enumerate(recipe['steps'], 1))
    return output


def process(db, event, call, work, check=True):
    validate_event(event)
    update_id = event['provider_update_id']
    identity = f'recipe-{update_id}'
    event_json = encoded(event).decode()
    with db:
        db.execute('BEGIN IMMEDIATE')
        previous = db.execute('SELECT event,disposition FROM processed_updates WHERE update_id=?', (update_id,)).fetchone()
        if previous and previous[0] != event_json:
            raise ValueError('conflicting provider update')
        if not previous:
            row = db.execute('SELECT summary FROM conversation_summaries WHERE user_id=?', (event['user_id'],)).fetchone()
            history = db.execute('SELECT request_summary FROM recipes WHERE user_id=? ORDER BY rowid DESC LIMIT 3', (event['user_id'],)).fetchall()
            context = {'summary': row[0] if row else '', 'recent_recipes': [x[0] for x in history]}
            db.execute('INSERT INTO processed_updates VALUES(?,?,?,?)', (update_id, 'pending', identity, event_json))
            db.execute('INSERT INTO recipes(recipe_id,user_id,request_summary,publication_outcome,context) VALUES(?,?,?,?,?)',
                       (identity, event['user_id'], event['text'], 'pending', encoded(context).decode()))
            db.execute('INSERT OR REPLACE INTO conversation_summaries VALUES(?,?)', (event['user_id'], event['text'][:512]))
    row = db.execute('SELECT model_response,relay_outcome,context FROM recipes WHERE recipe_id=?', (identity,)).fetchone()
    recipe = json.loads(row[0]) if row[0] else call('model', {'idempotency_key': identity, 'user_id': event['user_id'], 'text': event['text'], 'context': json.loads(row[2])})
    validate_recipe(recipe)
    version = db.execute('SELECT version FROM schema_meta').fetchone()[0]
    markdown = render(recipe, version)
    with db:
        db.execute('UPDATE recipes SET model_response=?,rendered_content=? WHERE recipe_id=?', (encoded(recipe).decode(), markdown, identity))
        if version == 2:
            db.execute('UPDATE recipes SET recipe_summary=? WHERE recipe_id=?', (recipe['summary'], identity))
        if not previous or previous[1] != 'completed':
            db.execute('INSERT OR REPLACE INTO conversation_summaries VALUES(?,?)', (event['user_id'], recipe['summary']))
    directory = work / 'content' / 'recipes'
    directory.mkdir(parents=True, exist_ok=True)
    (directory / (identity + '.md')).write_text(markdown)
    (directory / '_index.md').write_text('+++\ntitle = "Recipes"\n+++\n\nFamily recipes.\n')
    if check:
        subprocess.run([__import__('sys').executable, 'check.py'], cwd=work, check=True, capture_output=True)
    if not row[1]:
        outcome = call('telegram_relay', {'idempotency_key': identity, 'user_id': event['user_id'], 'text': f"{recipe['title']}: content/recipes/{identity}.md"})
        if set(outcome) != {'status', 'message_id'} or outcome['status'] != 'delivered':
            raise ValueError('relay delivery unsuccessful')
        bounded_text(outcome['message_id'], 128)
        with db:
            db.execute('UPDATE recipes SET relay_outcome=? WHERE recipe_id=?', (encoded(outcome).decode(), identity))
    with db:
        # The host imports this work tree on successful exit; do not claim publication.
        db.execute('UPDATE recipes SET publication_outcome=? WHERE recipe_id=?', ('proposal_ready', identity))
        db.execute('UPDATE processed_updates SET disposition=? WHERE update_id=?', ('completed', update_id))
    return identity


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--migrate', action='store_true')
    parser.add_argument('--rollback-fixture', action='store_true')
    args = parser.parse_args()
    db = connect('/var/lib/hephaestus/cooking.sqlite3')
    if args.migrate or args.rollback_fixture:
        migrate(db, args.rollback_fixture)
        return
    control = Path('/run/hephaestus')
    context = json.loads((control / 'context.json').read_bytes())
    if context['git_ref'] != 'refs/heads/main' or not context['commit_sha']:
        raise ValueError('exact blog target required')
    if not (control / 'mailbox-body').exists():
        # An installation probe can validate the release without inventing recipe work.
        print('{"disposition":"installed"}')
        return
    raw = (control / 'mailbox-body').read_bytes()
    if len(raw) > 8192:
        raise ValueError('oversized event')
    parameters = json.loads((control / 'parameters.json').read_bytes())
    identity = process(db, json.loads(raw), Broker(parameters), Path('/workspace/work'))
    print(json.dumps({'result_identity': identity, 'disposition': 'proposal_ready'}))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # Provider payloads, bearer bytes and subprocess diagnostics stay out of logs.
        frame = error.__traceback__
        stage = 'startup'
        while frame:
            if frame.tb_frame.f_code.co_filename == __file__:
                stage = frame.tb_frame.f_code.co_name + ':' + str(frame.tb_lineno)
            frame = frame.tb_next
        print('cooking request failed: ' + type(error).__name__ + ' at ' + stage,
              file=__import__('sys').stderr)
        raise SystemExit(1)
