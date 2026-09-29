import assert from 'node:assert/strict';
import { test } from 'node:test';

import { crateGuideDependencies, crateGuideHref } from '../src/guide-dependencies.mjs';

const guides = [
  { id: 'guide-a', path: 'crates/a/README.md', crateId: 'crate-a' },
  { id: 'guide-b', path: 'crates/b/README.md', crateId: 'crate-b' },
  { id: 'guide-c', path: 'crates/c/README.md', crateId: 'crate-c' },
];

const crates = {
  packages: [
    { id: 'crate-a', name: 'crate-a' },
    { id: 'crate-b', name: 'crate-b' },
    { id: 'crate-c', name: 'crate-c' },
  ],
  dependencies: [
    { from: 'crate-a', to: 'crate-b', kinds: ['normal'], optional: false, targets: ['all'] },
    { from: 'crate-a', to: 'crate-b', kinds: ['dev'], optional: true, targets: ['cfg(unix)'] },
    { from: 'crate-c', to: 'crate-a', kinds: ['build'], optional: false, targets: ['all'] },
  ],
};

test('crate guide dependencies keep outgoing and incoming directions distinct', () => {
  const relations = crateGuideDependencies(crates, guides, 'crate-a');

  assert.deepEqual(relations.dependsOn.map(({ id }) => id), ['crate-b']);
  assert.deepEqual(relations.usedBy.map(({ id }) => id), ['crate-c']);
  assert.equal(relations.usedBy[0].annotation, 'build');
});

test('crate guide dependencies deduplicate mixed required and conditional edges accurately', () => {
  const [{ annotation, href }] = crateGuideDependencies(crates, guides, 'crate-a').dependsOn;

  assert.equal(annotation, 'dev, normal');
  assert.equal(href, '#/guide/b');
});

test('crate guide hrefs link only crates with generated guide entries', () => {
  assert.equal(crateGuideHref('crate-b', guides), '#/guide/b');
  assert.equal(crateGuideHref('missing', guides), null);
});
