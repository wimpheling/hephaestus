import assert from 'node:assert/strict';
import { test } from 'node:test';

import { hasMermaidFence, mermaidBlockMarkup } from '../src/mermaid.mjs';

test('recognizes Mermaid fences without treating ordinary code as diagrams', () => {
  assert.equal(hasMermaidFence('```mermaid\nflowchart LR\n```'), true);
  assert.equal(hasMermaidFence('```rust\nlet value = 1;\n```'), false);
  assert.equal(hasMermaidFence('```\nflowchart LR\n```'), false);
});

test('keeps Mermaid source escaped and visible before rendering', () => {
  const markup = mermaidBlockMarkup('flowchart LR\nA[<source>] --> B');
  assert.match(markup, /class="mermaid-diagram"/);
  assert.match(markup, /&lt;source&gt;/);
  assert.doesNotMatch(markup, /<source>/);
});
