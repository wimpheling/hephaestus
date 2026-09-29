export const ROOT_GUIDE_ID = 'root';

function decodeSegment(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/** Parse the hash routes used by the code surface without touching browser state. */
export function parseHash(hash) {
  const raw = String(hash || '').replace(/^#/, '') || '/guide';
  const marker = raw.indexOf('?');
  const path = marker < 0 ? raw : raw.slice(0, marker);
  const query = marker < 0 ? '' : raw.slice(marker + 1);
  const segments = path.replace(/^\//, '').split('/');
  const route = segments[0];
  if (route === 'guide') return { route, guideId: segments[1] ? decodeSegment(segments[1]) : ROOT_GUIDE_ID };
  if (route === 'crates') {
    const params = new URLSearchParams(query);
    return {
      route,
      crateContext: params.get('context') || 'all',
      crateLayer: params.get('layer') || 'all',
      crateSearch: params.get('search') || '',
    };
  }
  if (route === 'grpc') {
    const params = new URLSearchParams(query);
    return {
      route,
      selectedService: params.get('service') || null,
      selectedMethod: params.get('method') || null,
      selectedMessage: params.get('message') || null,
    };
  }
  return { route: 'crates', crateContext: 'all', crateLayer: 'all', crateSearch: '' };
}

function splitDestination(destination) {
  const value = String(destination || '').trim();
  const marker = value.search(/[?#]/);
  if (marker < 0) return { path: value, suffix: '' };
  return { path: value.slice(0, marker), suffix: value.slice(marker) };
}

/**
 * Normalize a repository-relative path without allowing it to escape the
 * repository. The browser cannot read repository files, so this gives guide
 * links a stable path to use for either a guide route or a source link.
 */
export function normalizeRepositoryPath(path, basePath = '') {
  const value = String(path || '').trim();
  if (!value || value.includes('\\') || /^[a-z][a-z\d+.-]*:/i.test(value) || value.startsWith('//')) return null;
  const pathSegments = value.startsWith('/') ? [] : String(basePath).split('/').slice(0, -1);
  for (const segment of value.replace(/^\/+/, '').split('/')) {
    if (!segment || segment === '.') continue;
    if (segment === '..') {
      if (!pathSegments.length) return null;
      pathSegments.pop();
    } else {
      pathSegments.push(segment);
    }
  }
  return pathSegments.join('/');
}

function directoryFor(path) {
  const normalized = String(path || '').replace(/\\/g, '/').replace(/\/+$/, '');
  if (!normalized) return '';
  if (normalized === 'README.md') return '';
  return normalized.endsWith('/README.md') ? normalized.slice(0, -'/README.md'.length) : normalized;
}

function guideForPath(path, guides) {
  const normalized = String(path || '').replace(/\/+$/, '');
  const entries = Array.isArray(guides) ? guides : guides?.guides || [];
  return entries.find((guide) => {
    const guidePath = String(guide.path || '').replace(/\\/g, '/');
    return guidePath === normalized || directoryFor(guidePath) === normalized;
  });
}

function isSafeExternalDestination(destination) {
  return /^(?:https?:\/\/|mailto:|tel:)/i.test(destination);
}

function isSafeSourceBase(sourceBase) {
  return /^(?:https?:\/\/|mailto:|tel:)/i.test(sourceBase) && sourceBase.endsWith('/');
}

function sourceBaseForDestination(base, destinationPath, directory) {
  if (!isSafeSourceBase(String(base || ''))) return '#';
  const sourceKind = directory ? 'tree' : 'blob';
  const marker = String(base).indexOf('/blob/');
  const sourceBase = marker >= 0 ? `${String(base).slice(0, marker)}/${sourceKind}/${String(base).slice(marker + '/blob/'.length)}` : base;
  return `${sourceBase}${destinationPath}`;
}

/**
 * Turn an authored README destination into a route, an external source link,
 * or an inert value for unsafe input. Known guide directories and READMEs use
 * the local guide route; other repository files use the repository source.
 */
export function guideLinkHref(
  destination,
  sourcePath,
  guides,
  sourceBase = guides?.sourceBase,
  sourceTreeBase = guides?.sourceTreeBase,
) {
  const value = String(destination || '').trim();
  if (!value) return '#';
  if (value.startsWith('#')) return value;
  if (isSafeExternalDestination(value)) return value;
  if (/^[a-z][a-z\d+.-]*:/i.test(value) || value.startsWith('//')) return '#';

  const { path, suffix } = splitDestination(value);
  const normalized = normalizeRepositoryPath(path, sourcePath);
  if (!normalized) return '#';
  const guide = guideForPath(normalized, guides);
  if (guide) return `#/guide/${encodeURIComponent(guide.id)}${suffix}`;
  const authoredDirectory = (guides?.sourceDirectories || []).includes(normalized);
  const directory = path.endsWith('/') || authoredDirectory;
  const directoryBase = sourceTreeBase || String(sourceBase || '').replace('/blob/', '/tree/');
  const href = sourceBaseForDestination(directory ? directoryBase : sourceBase, normalized, directory);
  return href === '#' ? '#' : `${href}${suffix}`;
}
