import assert from 'node:assert/strict';
import { test } from 'node:test';

import { guideLinkHref, normalizeRepositoryPath, parseHash, ROOT_GUIDE_ID } from '../src/navigation.mjs';

const guides = [
  { id: ROOT_GUIDE_ID, path: 'README.md' },
  { id: 'core', path: 'crates/heph-core/README.md' },
  { id: 'runtime', path: 'crates/heph-core/runtime/README.md' },
];
const sourceBase = 'https://github.com/wimpheling/hephaestus/blob/feat/code-surface/';
const sourceTreeBase = 'https://github.com/wimpheling/hephaestus/tree/feat/code-surface/';
guides.sourceDirectories = ['crates/heph-core/auth/secret', 'crates/heph-std/runtime/vm/libkrun'];

test('guide hash routes select the root and preserve crate context filters', () => {
  assert.deepEqual(parseHash('#/guide'), { route: 'guide', guideId: ROOT_GUIDE_ID });
  assert.deepEqual(parseHash(''), { route: 'guide', guideId: ROOT_GUIDE_ID });
  assert.deepEqual(parseHash('#/crates?context=runtime'), {
    route: 'crates',
    crateContext: 'runtime',
    crateLayer: 'all',
    crateSearch: '',
  });
});

test('README links resolve to guide routes or validated source links', () => {
  assert.equal(guideLinkHref('../heph-core/', 'crates/heph-app/README.md', guides), '#/guide/core');
  assert.equal(guideLinkHref('../heph-core/#overview', 'crates/heph-app/README.md', guides), '#/guide/core#overview');
  assert.equal(guideLinkHref('../runtime/', 'crates/heph-core/auth/README.md', guides), '#/guide/runtime');
  assert.equal(
    guideLinkHref('crates/heph-core/auth/secret/', 'README.md', guides, sourceBase, sourceTreeBase),
    `${sourceTreeBase}crates/heph-core/auth/secret`,
  );
  assert.equal(
    guideLinkHref('crates/heph-std/runtime/vm/libkrun', 'README.md', guides, sourceBase, sourceTreeBase),
    `${sourceTreeBase}crates/heph-std/runtime/vm/libkrun`,
  );
  assert.equal(
    guideLinkHref('../../../docs/application.md', 'crates/heph-core/runtime/README.md', guides),
    '#',
  );
  assert.equal(
    guideLinkHref('../../../docs/application.md', 'crates/heph-core/runtime/README.md', guides, sourceBase),
    `${sourceBase}docs/application.md`,
  );
  assert.equal(normalizeRepositoryPath('../../../docs/application.md', 'crates/heph-core/runtime/README.md'), 'docs/application.md');
});

test('unsafe and protocol-relative destinations are inert while web links remain usable', () => {
  assert.equal(guideLinkHref('javascript:alert(1)', 'README.md', guides), '#');
  assert.equal(guideLinkHref('//attacker.example/readme', 'README.md', guides), '#');
  assert.equal(guideLinkHref('docs/application.md', 'README.md', guides, 'javascript:alert(1)/'), '#');
  assert.equal(guideLinkHref('https://example.com/docs?q=1&x=2', 'README.md', guides), 'https://example.com/docs?q=1&x=2');
  assert.equal(guideLinkHref('#section', 'README.md', guides), '#section');
});
