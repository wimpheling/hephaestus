import assert from 'node:assert/strict';
import { test } from 'node:test';

import { guideHref, guideLinkHref, normalizeRepositoryPath, parseHash, ROOT_GUIDE_ID, resolveGuide, resolveGuideOrRoot } from '../src/navigation.mjs';

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
  assert.equal(guideLinkHref('../heph-core/', 'crates/heph-app/README.md', guides), '#/guide/heph-core');
  assert.equal(guideLinkHref('../heph-core/#overview', 'crates/heph-app/README.md', guides), '#/guide/heph-core#overview');
  assert.equal(guideLinkHref('../runtime/', 'crates/heph-core/auth/README.md', guides), '#/guide/heph-core/runtime');
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

test('canonical guide routes include the complete crate path and resolve duplicate names', () => {
  const duplicateGuides = [
    { id: 'directory-666f726765', path: 'crates/heph-core/forge/README.md' },
    { id: 'directory-7374642d666f726765', path: 'crates/heph-std/forge/README.md' },
  ];
  assert.equal(guideHref(duplicateGuides[0]), '#/guide/heph-core/forge');
  assert.equal(guideHref(duplicateGuides[1]), '#/guide/heph-std/forge');
  assert.equal(resolveGuide(duplicateGuides, 'heph-core/forge'), duplicateGuides[0]);
  assert.equal(resolveGuide(duplicateGuides, 'heph-std/forge'), duplicateGuides[1]);
  assert.equal(resolveGuide(duplicateGuides, 'directory-666f726765'), duplicateGuides[0]);
  assert.deepEqual(parseHash('#/guide/heph-core/auth/identity/domain'), {
    route: 'guide',
    guideId: 'heph-core/auth/identity/domain',
  });
  assert.deepEqual(parseHash('#/guide/heph-core/auth/identity/domain/'), {
    route: 'guide',
    guideId: 'heph-core/auth/identity/domain',
  });
});

test('legacy authored and generated IDs map to the canonical route', () => {
  const legacyGuides = [
    { id: 'root', path: 'README.md' },
    { id: 'core', path: 'crates/heph-core/README.md' },
    { id: 'directory-686570682d636f7265', path: 'crates/heph-core/auth/README.md' },
  ];
  assert.equal(guideHref(resolveGuide(legacyGuides, 'root')), '#/guide');
  assert.equal(guideHref(resolveGuide(legacyGuides, 'core')), '#/guide/heph-core');
  assert.equal(guideHref(resolveGuide(legacyGuides, 'directory-686570682d636f7265')), '#/guide/heph-core/auth');
  assert.equal(resolveGuideOrRoot(legacyGuides, 'missing-guide'), legacyGuides[0]);
});

test('unsafe and protocol-relative destinations are inert while web links remain usable', () => {
  assert.equal(guideLinkHref('javascript:alert(1)', 'README.md', guides), '#');
  assert.equal(guideLinkHref('//attacker.example/readme', 'README.md', guides), '#');
  assert.equal(guideLinkHref('docs/application.md', 'README.md', guides, 'javascript:alert(1)/'), '#');
  assert.equal(guideLinkHref('https://example.com/docs?q=1&x=2', 'README.md', guides), 'https://example.com/docs?q=1&x=2');
  assert.equal(guideLinkHref('#section', 'README.md', guides), '#section');
});
