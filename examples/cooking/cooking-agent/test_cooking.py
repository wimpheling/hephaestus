import importlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
import struct
import subprocess
import sys
import cooking_agent as agent

spec = importlib.util.spec_from_file_location('relay', Path(__file__).resolve().parents[1] / 'telegram-relay/relay.py')
relay_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(relay_module)


class Journey(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.work = self.root / 'work'
        shutil.copytree(Path(__file__).resolve().parents[1] / 'cooking-blog', self.work,
                        ignore=shutil.ignore_patterns('.git', 'public', '__pycache__'))
        self.db = agent.connect(self.root / 'state.db')
        self.addCleanup(self.db.close)
        self.relay = relay_module.Relay(self.root / 'relay.db', b'local-fixture-credential')
        self.addCleanup(self.relay.db.close)
        self.calls = []
        self.event = {'provider_update_id': 42, 'user_id': 'alice', 'command': 'recipe', 'text': 'pasta'}

    def call(self, slot, body):
        self.calls.append((slot, body))
        if slot == 'model':
            return {'title': 'Family pasta', 'summary': 'A quick family supper.',
                    'ingredients': ['pasta', 'tomatoes'], 'steps': ['Boil pasta.', 'Add tomatoes.']}
        status, response = self.relay.deliver('Bearer local-fixture-credential', agent.encoded(body))
        self.assertEqual(status, 200)
        return response

    def test_complete_request_replay_and_restart(self):
        self.assertEqual(agent.process(self.db, self.event, self.call, self.work), 'recipe-42')
        self.assertEqual(len(self.calls), 2)
        agent.process(self.db, self.event, self.call, self.work)
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.db.execute('SELECT count(*) FROM recipes').fetchone()[0], 1)
        self.db.close()
        self.db = agent.connect(self.root / 'state.db')
        self.addCleanup(self.db.close)
        self.work = self.root / 'restarted-work'
        shutil.copytree(Path(__file__).resolve().parents[1] / 'cooking-blog', self.work,
                        ignore=shutil.ignore_patterns('.git', 'public', '__pycache__'))
        agent.process(self.db, self.event, self.call, self.work)
        self.assertTrue((self.work / 'content/recipes/recipe-42.md').is_file())
        self.assertEqual(len(self.calls), 2)
        second = dict(self.event, provider_update_id=43)
        agent.process(self.db, second, self.call, self.work)
        self.assertEqual(self.calls[2][1]['context']['summary'], 'A quick family supper.')
        self.assertEqual(self.relay.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 2)

    def test_crash_after_relay_effect_retries_same_key(self):
        def crash(slot, body):
            outcome = self.call(slot, body)
            if slot == 'telegram_relay':
                raise RuntimeError('simulated lost response')
            return outcome
        with self.assertRaises(RuntimeError):
            agent.process(self.db, self.event, crash, self.work)
        agent.process(self.db, self.event, self.call, self.work)
        self.assertEqual([x[0] for x in self.calls], ['model', 'telegram_relay', 'telegram_relay'])
        self.assertEqual(self.relay.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 1)

    def test_update_rollback_idempotency_and_v2_output(self):
        agent.process(self.db, self.event, self.call, self.work)
        with self.assertRaises(ValueError):
            agent.migrate(self.db, fail=True)
        self.assertEqual(self.db.execute('SELECT version FROM schema_meta').fetchone()[0], 1)
        self.assertNotIn('recipe_summary', [x[1] for x in self.db.execute('PRAGMA table_info(recipes)')])
        agent.migrate(self.db)
        agent.migrate(self.db)
        agent.process(self.db, self.event, self.call, self.work)
        self.assertIn('A quick family supper.', (self.work / 'content/recipes/recipe-42.md').read_text())
        self.assertEqual(self.db.execute('SELECT recipe_summary FROM recipes').fetchone()[0], 'A quick family supper.')

    def test_conflicting_replay_and_unauthorized_event(self):
        agent.process(self.db, self.event, self.call, self.work)
        for event in (dict(self.event, text='different'), dict(self.event, user_id='mallory')):
            with self.assertRaises(ValueError):
                agent.process(self.db, event, self.call, self.work)
        self.assertEqual(len(self.calls), 2)

    def test_malformed_model_cannot_render_or_send(self):
        with self.assertRaises(ValueError):
            agent.process(self.db, self.event, lambda *_: {'title': 'bad'}, self.work)
        self.assertFalse((self.work / 'content/recipes/recipe-42.md').exists())
        self.assertEqual(self.relay.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 0)

    def test_retry_recovers_recipe_after_state_commit_before_model_call(self):
        def interrupted_model(slot, _body):
            self.assertEqual(slot, 'model')
            raise RuntimeError('simulated model-call interruption')

        with self.assertRaises(RuntimeError):
            agent.process(self.db, self.event, interrupted_model, self.work)
        self.assertEqual(
            self.db.execute(
                'SELECT disposition FROM processed_updates WHERE update_id=?', (42,)
            ).fetchone()[0],
            'pending',
        )
        self.assertEqual(self.db.execute('SELECT count(*) FROM recipes').fetchone()[0], 1)
        agent.process(self.db, self.event, self.call, self.work)
        self.assertEqual(self.db.execute('SELECT count(*) FROM recipes').fetchone()[0], 1)
        self.assertEqual(self.db.execute('SELECT count(*) FROM processed_updates').fetchone()[0], 1)
        self.assertEqual(self.relay.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 1)

    def test_relay_auth_bounds_and_conflict(self):
        request = {'idempotency_key': 'recipe-1', 'user_id': 'bob', 'text': 'hello'}
        raw = agent.encoded(request)
        self.assertEqual(self.relay.deliver('invalid', raw)[0], 401)
        self.assertEqual(self.relay.deliver('Bearer local-fixture-credential', b'x' * 8193)[0], 400)
        self.assertEqual(self.relay.deliver('Bearer local-fixture-credential', raw)[0], 200)
        self.assertEqual(self.relay.deliver('Bearer local-fixture-credential', agent.encoded(dict(request, text='different')))[0], 409)

    def test_relay_rejects_malformed_payloads_without_ledger_effect(self):
        invalid_requests = (
            {},
            {'idempotency_key': 'recipe-2', 'user_id': 'mallory', 'text': 'hello'},
            {'idempotency_key': 'recipe-2', 'user_id': 'alice', 'text': ''},
            {'idempotency_key': 'recipe-2', 'user_id': 'alice', 'text': 'x' * 2049},
            {'idempotency_key': 'recipe\n2', 'user_id': 'alice', 'text': 'hello'},
            {'idempotency_key': 'recipe-2', 'user_id': 'alice', 'text': 'hello', 'extra': True},
        )
        for request in invalid_requests:
            with self.subTest(request=request):
                status, _ = self.relay.deliver(
                    'Bearer local-fixture-credential', agent.encoded(request)
                )
                self.assertEqual(status, 400)
        self.assertEqual(self.relay.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 0)

    def test_relay_replay_and_conflict_survive_database_reopen(self):
        request = {'idempotency_key': 'recipe-restart', 'user_id': 'alice', 'text': 'soup'}
        status, original = self.relay.deliver(
            'Bearer local-fixture-credential', agent.encoded(request)
        )
        self.assertEqual(status, 200)
        self.relay.db.close()
        reopened = relay_module.Relay(self.root / 'relay.db', b'local-fixture-credential')
        try:
            replay_status, replay = reopened.deliver(
                'Bearer local-fixture-credential', agent.encoded(request)
            )
            self.assertEqual(replay_status, 200)
            self.assertEqual(replay, original)
            conflict_status, _ = reopened.deliver(
                'Bearer local-fixture-credential',
                agent.encoded(dict(request, text='different')),
            )
            self.assertEqual(conflict_status, 409)
            self.assertEqual(
                reopened.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 1
            )
        finally:
            reopened.db.close()

    def test_relay_credential_rotation_preserves_history_and_fences_old_key(self):
        request = {'idempotency_key': 'recipe-rotation', 'user_id': 'bob', 'text': 'stew'}
        status, original = self.relay.deliver(
            'Bearer local-fixture-credential', agent.encoded(request)
        )
        self.assertEqual(status, 200)
        self.relay.db.close()
        rotated = relay_module.Relay(self.root / 'relay.db', b'rotated-fixture-credential')
        try:
            old_status, _ = rotated.deliver(
                'Bearer local-fixture-credential', agent.encoded(request)
            )
            self.assertEqual(old_status, 401)
            replay_status, replay = rotated.deliver(
                'Bearer rotated-fixture-credential', agent.encoded(request)
            )
            self.assertEqual(replay_status, 200)
            self.assertEqual(replay, original)
            self.assertEqual(
                rotated.db.execute('SELECT count(*) FROM deliveries').fetchone()[0], 1
            )
        finally:
            rotated.db.close()

    def test_old_replay_does_not_rewind_context(self):
        agent.process(self.db, self.event, self.call, self.work)
        with self.db:
            self.db.execute("UPDATE conversation_summaries SET summary='newer recipe' WHERE user_id='alice'")
        agent.process(self.db, self.event, self.call, self.work)
        self.assertEqual(self.db.execute("SELECT summary FROM conversation_summaries WHERE user_id='alice'").fetchone()[0], 'newer recipe')

    def test_unicode_title_and_delimiter_are_valid_toml(self):
        recipe = {'title': 'Soup 🍲 +++', 'summary': 'Delicious.', 'ingredients': ['water'], 'steps': ['Boil.']}
        def call(slot, body):
            return recipe if slot == 'model' else self.call(slot, body)
        agent.process(self.db, self.event, call, self.work)

    @unittest.skipUnless(os.environ.get('COOKING_HUGO_BINARY'), 'optional pinned Hugo executable')
    def test_pinned_hugo_builds_real_html(self):
        agent.process(self.db, self.event, self.call, self.work)
        subprocess.run([os.environ['COOKING_HUGO_BINARY'], '--destination', str(self.root / 'html'),
                        '--cacheDir', str(self.root / 'hugo-cache')], cwd=self.work, check=True, capture_output=True)
        page = self.root / 'html/recipes/recipe-42/index.html'
        self.assertIn('Family pasta', page.read_text())

    def test_broker_uses_actual_framed_wire_and_rule_placeholder(self):
        rule = '00000000-0000-0000-0000-000000000002'
        broker = object.__new__(agent.Broker)
        broker.parameters = {'model_rule_id': rule}
        broker.context = {'run_id': '00000000-0000-0000-0000-000000000003'}
        response = agent.encoded({'status': 'succeeded', 'body': list(b'{"title":"ok"}')})
        class Stream:
            def __init__(self):
                self.data = struct.pack('!I', len(response)) + response
                self.sent = b''
            def __enter__(self): return self
            def __exit__(self, *_): pass
            def settimeout(self, _): pass
            def connect(self, address): self.address = address
            def sendall(self, data): self.sent = data
            def recv(self, length):
                result, self.data = self.data[:length], self.data[length:]
                return result
        stream = Stream()
        with patch.object(agent.socket, 'socket', return_value=stream), patch.object(Path, 'read_bytes', return_value=b'x' * 32):
            self.assertEqual(broker('model', {'text': 'pasta'}), {'title': 'ok'})
        self.assertEqual(stream.address, (2, 19001))
        wire = json.loads(stream.sent[4:])
        self.assertEqual(struct.unpack('!I', stream.sent[:4])[0], len(stream.sent) - 4)
        self.assertEqual(wire['operation'], 'https_v1')
        request = json.loads(bytes(wire['body']))
        self.assertEqual(request['headers'][0]['value'], 'Bearer heph-placeholder:v1:' + rule)
        self.assertEqual(request['path_and_query'], '/v1/recipes')


def load_tests(loader, tests, pattern):
    """Include the sibling guest safety regressions in application discovery."""
    tests_root = Path(__file__).resolve().parents[1] / 'tests'
    modules = (
        ('cooking_guest_crash_regression', tests_root / 'guest_crash_regression.py'),
        ('cooking_guest_confinement_probe', tests_root / 'guest_confinement_probe.py'),
    )
    for module_name, module_path in modules:
        if module_name == 'cooking_guest_confinement_probe':
            runtime_path = tests_root / 'guest_confinement.py'
            runtime_spec = importlib.util.spec_from_file_location(
                'guest_confinement', runtime_path
            )
            if runtime_spec is None or runtime_spec.loader is None:
                raise ImportError(f'cannot load {runtime_path}')
            runtime_module = importlib.util.module_from_spec(runtime_spec)
            sys.modules['guest_confinement'] = runtime_module
            runtime_spec.loader.exec_module(runtime_module)
        spec = importlib.util.spec_from_file_location(module_name, module_path)
        if spec is None or spec.loader is None:
            raise ImportError(f'cannot load {module_path}')
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        tests.addTests(loader.loadTestsFromModule(module))
    return tests


if __name__ == '__main__':
    unittest.main()
