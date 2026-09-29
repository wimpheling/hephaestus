/**
 * The guide hierarchy is intentionally opened at its architectural seams so
 * the first view is useful without expanding every generated crate page.
 */
export const DEFAULT_OPEN_GUIDE_IDS = Object.freeze([
  'core',
]);

function childrenByParent(guides) {
  const children = new Map();
  for (const guide of guides || []) {
    if (!guide.parent) continue;
    const siblings = children.get(guide.parent) || [];
    siblings.push(guide);
    children.set(guide.parent, siblings);
  }
  return children;
}

/** Return the expansion state used on the first guide render. */
export function defaultGuideExpansion(guides) {
  const children = childrenByParent(guides);
  return new Set(DEFAULT_OPEN_GUIDE_IDS.filter((id) => children.has(id)));
}

/** Return every parent between a guide and the root, nearest parent first. */
export function guideAncestors(guides, selectedId) {
  const byId = new Map((guides || []).map((guide) => [guide.id, guide]));
  const ancestors = new Set();
  let current = byId.get(selectedId);
  while (current?.parent && !ancestors.has(current.parent)) {
    ancestors.add(current.parent);
    current = byId.get(current.parent);
  }
  return ancestors;
}

/**
 * Preserve manual expansion while opening the path required to show the
 * selected guide. A copy is returned so callers can safely keep the previous
 * state while a route is being resolved.
 */
export function expansionForGuide(guides, selectedId, expandedIds = null, openSelected = true) {
  const expanded = new Set(expandedIds || defaultGuideExpansion(guides));
  for (const ancestor of guideAncestors(guides, selectedId)) expanded.add(ancestor);
  if (openSelected && (guides || []).some((child) => child.parent === selectedId)) {
    expanded.add(selectedId);
  }
  return expanded;
}

/** Toggle one node without mutating the caller's set. */
export function toggleGuideExpansion(expandedIds, guideId) {
  const next = new Set(expandedIds || []);
  if (next.has(guideId)) next.delete(guideId);
  else next.add(guideId);
  return next;
}
