import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  defaultGuideExpansion,
  expansionForGuide,
  guideAncestors,
  toggleGuideExpansion,
} from '../src/guide-tree.mjs';

const guides = [
  { id: 'root' },
  { id: 'core', parent: 'root' },
  { id: 'std', parent: 'root' },
  { id: 'runtime', parent: 'core' },
  { id: 'deep-crate', parent: 'runtime' },
  { id: 'std-crate', parent: 'std' },
];

test('default guide expansion opens the architecture seams and leaves crate lists closed', () => {
  assert.deepEqual([...defaultGuideExpansion(guides)].sort(), ['core'].sort());
});

test('deep guide routes expand every ancestor while preserving manual expansion', () => {
  assert.deepEqual([...guideAncestors(guides, 'deep-crate')], ['runtime', 'core', 'root']);
  const expanded = expansionForGuide(guides, 'deep-crate', new Set(['std']));
  assert.deepEqual([...expanded].sort(), ['core', 'root', 'runtime', 'std'].sort());
});

test('opening a branch guide reveals only its direct children and manual collapse persists', () => {
  const opened = expansionForGuide(guides, 'runtime', new Set(['core']), true);
  assert.deepEqual([...opened].sort(), ['core', 'root', 'runtime'].sort());

  const manuallyCollapsed = expansionForGuide(guides, 'runtime', new Set(['core']), false);
  assert.deepEqual([...manuallyCollapsed].sort(), ['core', 'root'].sort());
});

test('expansion toggles are immutable so route navigation can retain the set', () => {
  const original = new Set(['root']);
  const collapsed = toggleGuideExpansion(original, 'root');
  const reopened = toggleGuideExpansion(collapsed, 'root');
  assert.deepEqual([...original], ['root']);
  assert.deepEqual([...collapsed], []);
  assert.deepEqual([...reopened], ['root']);
});
