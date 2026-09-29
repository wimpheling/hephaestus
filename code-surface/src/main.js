import cytoscape from 'cytoscape';
import './styles.css';

const app = document.querySelector('#app');
const baseUrl = (import.meta.env.BASE_URL || '/').replace(/\/$/, '') + '/';

const state = {
  route: 'crates',
  crates: null,
  grpc: null,
  loading: true,
  error: null,
  crateSearch: '',
  crateContext: 'all',
  crateLayer: 'all',
  selectedCrate: null,
  grpcSearch: '',
  grpcPackage: 'all',
  selectedService: null,
  selectedMethod: null,
  selectedMessage: null,
  graph: null,
  grpcGraph: null,
};

const esc = (value) => String(value ?? '')
  .replaceAll('&', '&amp;')
  .replaceAll('<', '&lt;')
  .replaceAll('>', '&gt;')
  .replaceAll('"', '&quot;')
  .replaceAll("'", '&#039;');

const slug = (value) => String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, '-');
const shortName = (value) => String(value || '').split('::').at(-1).split('.').at(-1);
const labelFor = (value, max = 34) => {
  const text = String(value || '');
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
};
const unique = (values) => [...new Set(values.filter(Boolean))].sort((a, b) => a.localeCompare(b));
const findById = (items, id) => items?.find((item) => item.id === id);
const isCrateOverview = () => state.crateContext === 'all' && !state.crateSearch.trim() && state.crateLayer === 'all';

function routeFromHash() {
  const raw = window.location.hash.slice(1) || '/crates';
  const [path, query] = raw.split('?');
  const route = path.replace(/^\//, '').split('/')[0];
  if (route === 'grpc') {
    const params = new URLSearchParams(query || '');
    if (params.get('service')) state.selectedService = params.get('service');
    if (params.get('method')) state.selectedMethod = params.get('method');
    if (params.get('message')) state.selectedMessage = params.get('message');
    return 'grpc';
  }
  return 'crates';
}

function navigate(route) {
  window.location.hash = `#/${route}`;
}

function syncSelectionHash() {
  const params = new URLSearchParams();
  if (state.selectedService) params.set('service', state.selectedService);
  if (state.selectedMethod) params.set('method', state.selectedMethod);
  if (state.selectedMessage) params.set('message', state.selectedMessage);
  const suffix = params.toString() ? `?${params.toString()}` : '';
  window.history.replaceState(null, '', `#/${state.route}${suffix}`);
}

async function loadData() {
  state.loading = true;
  state.error = null;
  render();
  try {
    const [cratesResponse, grpcResponse] = await Promise.all([
      fetch(`${baseUrl}generated/crates.json`),
      fetch(`${baseUrl}generated/grpc.json`),
    ]);
    if (!cratesResponse.ok || !grpcResponse.ok) {
      const missing = [!cratesResponse.ok && 'crates.json', !grpcResponse.ok && 'grpc.json'].filter(Boolean).join(' and ');
      throw new Error(`Generated ${missing} ${missing.includes(' and ') ? 'are' : 'is'} unavailable.`);
    }
    const [crates, grpc] = await Promise.all([cratesResponse.json(), grpcResponse.json()]);
    if (crates.schemaVersion !== 1 || grpc.schemaVersion !== 1) throw new Error('The generated data uses an unsupported schema version.');
    state.crates = crates;
    state.grpc = grpc;
    state.loading = false;
    state.route = routeFromHash();
    render();
  } catch (error) {
    state.loading = false;
    state.error = error instanceof Error ? error.message : 'Unable to load generated data.';
    render();
  }
}

function render() {
  if (state.loading) {
    app.innerHTML = `<main class="loading-screen"><div class="brand-mark">◌</div><p>Reading the code surface…</p></main>`;
    return;
  }
  if (state.error) {
    app.innerHTML = `<main class="error-screen"><div class="brand-mark">!</div><p class="eyebrow">CODE SURFACE</p><h1>Generated data is not ready</h1><p class="muted">${esc(state.error)}</p><p class="hint">Run <code>npm run generate</code> from <code>code-surface/</code>, then reload.</p><button class="button primary" id="retry">Try again</button></main>`;
    document.querySelector('#retry')?.addEventListener('click', loadData);
    return;
  }

  state.graph?.destroy();
  state.graph = null;
  state.grpcGraph?.destroy();
  state.grpcGraph = null;
  app.innerHTML = `
    <div class="app-shell">
      <header class="topbar">
        <a class="wordmark" href="#/crates" aria-label="Code Surface home"><span class="logo-dot"></span><span>code<span class="wordmark-soft">/</span>surface</span></a>
        <nav class="main-nav" aria-label="Primary navigation">
          <a class="nav-link ${state.route === 'crates' ? 'active' : ''}" href="#/crates"><span class="nav-icon">▦</span>Crates</a>
          <a class="nav-link ${state.route === 'grpc' ? 'active' : ''}" href="#/grpc"><span class="nav-icon">⌘</span>gRPC</a>
        </nav>
        <div class="topbar-meta"><span class="live-dot"></span><span>generated view</span><span class="version-badge">v1</span></div>
      </header>
      <main class="content">${state.route === 'grpc' ? renderGrpcPage() : renderCratesPage()}</main>
      <footer class="footer"><span>Hephaestus code surface</span><span>Derived from source · schema v1</span></footer>
    </div>`;
  bindPageEvents();
  if (state.route === 'crates') renderCrateGraph();
  else renderGrpcGraph();
}

function renderCratesPage() {
  const packages = state.crates.packages || [];
  const deps = state.crates.dependencies || [];
  const contexts = unique(packages.map((item) => item.context));
  const layers = unique(packages.map((item) => item.layer));
  const overview = isCrateOverview();
  return `
    <section class="page-heading">
      <div><p class="eyebrow">WORKSPACE TOPOLOGY</p><h1>Crates</h1><p class="page-lede">A map of the workspace and the dependencies that hold it together.</p></div>
      <div class="stats"><div class="stat"><strong>${packages.length}</strong><span>packages</span></div><div class="stat"><strong>${deps.length}</strong><span>links</span></div><div class="stat"><strong>${contexts.length}</strong><span>contexts</span></div></div>
    </section>
    <section class="toolbar" aria-label="Crate filters">
      <label class="search-wrap"><span class="sr-only">Search crates</span><span class="search-icon">⌕</span><input id="crate-search" type="search" placeholder="Search packages…" value="${esc(state.crateSearch)}" /></label>
      <label class="select-wrap"><span class="sr-only">Filter by context</span><select id="crate-context"><option value="all">All contexts</option>${contexts.map((value) => `<option value="${esc(value)}" ${state.crateContext === value ? 'selected' : ''}>${esc(value)}</option>`).join('')}</select></label>
      <label class="select-wrap"><span class="sr-only">Filter by layer</span><select id="crate-layer"><option value="all">All layers</option>${layers.map((value) => `<option value="${esc(value)}" ${state.crateLayer === value ? 'selected' : ''}>${esc(value)}</option>`).join('')}</select></label>
      <span class="toolbar-note">Scroll to zoom · drag to pan · click a node</span>
    </section>
    <section class="surface-grid crates-grid">
      <div class="graph-card"><div class="card-heading"><div><p class="eyebrow">${overview ? 'CONTEXT OVERVIEW' : 'DEPENDENCY GRAPH'}</p><h2>${overview ? 'Workspace contexts' : 'Crate dependencies'}</h2><p class="graph-caption">${overview ? 'Click a context to open its crate subgraph.' : 'Showing packages matching the current filters.'}</p></div><div class="graph-actions">${overview ? '' : '<button class="overview-button" id="graph-overview">← overview</button>'}<button class="icon-button" id="graph-fit" title="Fit graph" aria-label="Fit graph">⌗</button><button class="icon-button" id="graph-zoom-in" title="Zoom in" aria-label="Zoom in">+</button><button class="icon-button" id="graph-zoom-out" title="Zoom out" aria-label="Zoom out">−</button></div></div><div id="crate-graph" class="graph-canvas" role="img" aria-label="Interactive workspace dependency graph"></div><div class="legend">${overview ? '<span><i class="legend-swatch context"></i>context</span><span><i class="legend-line"></i>cross-context dependency</span>' : '<span><i class="legend-swatch domain"></i>domain</span><span><i class="legend-swatch adapter"></i>adapter</span><span><i class="legend-swatch interface"></i>interface</span><span><i class="legend-swatch app"></i>application</span><span><i class="legend-line"></i>dependency</span>'}</div></div>
      <aside id="crate-details" class="detail-card" aria-live="polite">${overview ? renderContextOverviewDetails() : renderCrateDetails()}</aside>
    </section>`;
}

function renderCrateDetails() {
  const packages = state.crates?.packages || [];
  const deps = state.crates?.dependencies || [];
  const selected = findById(packages, state.selectedCrate);
  if (!selected) return `<div class="empty-detail"><span class="empty-icon">◈</span><p class="eyebrow">PACKAGE INSPECTOR</p><h2>Select a crate</h2><p>Click any node in the graph to inspect its manifest, targets, and direct dependencies.</p></div>`;
  const outgoing = deps.filter((dep) => dep.from === selected.id);
  const incoming = deps.filter((dep) => dep.to === selected.id);
  return `<div class="detail-top"><div><p class="eyebrow">PACKAGE INSPECTOR</p><h2>${esc(selected.name)}</h2></div><span class="layer-pill ${slug(selected.layer)}">${esc(selected.layer || selected.context || 'package')}</span></div>
    <p class="detail-description">${esc(selected.description || 'No package description.')}</p>
    <dl class="metadata"><div><dt>Manifest</dt><dd class="mono">${esc(selected.manifest || '—')}</dd></div><div><dt>Context</dt><dd>${esc(selected.context || '—')}</dd></div><div><dt>Targets</dt><dd>${selected.targets?.length || 0}</dd></div></dl>
    <div class="detail-section"><h3>Targets <span>${selected.targets?.length || 0}</span></h3>${selected.targets?.length ? `<ul class="compact-list">${selected.targets.map((target) => `<li><span class="target-icon">◇</span><span>${esc(target.name)}</span><small>${esc((target.kinds || []).join(', '))}</small></li>`).join('')}</ul>` : '<p class="muted">No targets declared.</p>'}</div>
    <div class="detail-section"><h3>Dependencies <span>${outgoing.length}</span></h3>${outgoing.length ? `<ul class="dependency-list">${outgoing.map((dep) => `<li><button class="text-button crate-link" data-crate="${esc(dep.to)}">${esc(shortName(dep.to))}</button><small>${esc((dep.kinds || []).join(', ') || dep.name || 'workspace')}${dep.optional ? ' · optional' : ''}${dep.targets?.length ? ` · ${esc(dep.targets.join(', '))}` : ''}</small></li>`).join('')}</ul>` : '<p class="muted">No outgoing workspace dependencies.</p>'}</div>
    <div class="detail-section"><h3>Used by <span>${incoming.length}</span></h3>${incoming.length ? `<ul class="dependency-list">${incoming.map((dep) => `<li><button class="text-button crate-link" data-crate="${esc(dep.from)}">${esc(shortName(dep.from))}</button><small>${esc(dep.name || 'workspace')}${dep.optional ? ' · optional' : ''}</small></li>`).join('')}</ul>` : '<p class="muted">No workspace dependants.</p>'}</div>`;
}

function renderContextOverviewDetails() {
  const contexts = unique((state.crates.packages || []).map((item) => item.context || 'uncategorized'));
  return `<div class="empty-detail"><span class="empty-icon">◌</span><p class="eyebrow">CONTEXT INSPECTOR</p><h2>Explore the workspace</h2><p>There are <strong>${contexts.length} contexts</strong> in this workspace. Click a context node to inspect its crates, targets, and dependencies.</p></div>`;
}

function renderCrateGraph() {
  const container = document.querySelector('#crate-graph');
  if (!container) return;
  if (isCrateOverview()) {
    renderContextGraph(container);
    return;
  }
  const query = state.crateSearch.toLowerCase().trim();
  const packages = (state.crates.packages || []).filter((item) => {
    const matchesSearch = !query || [item.name, item.id, item.description].some((value) => String(value || '').toLowerCase().includes(query));
    const matchesContext = state.crateContext === 'all' || item.context === state.crateContext;
    const matchesLayer = state.crateLayer === 'all' || item.layer === state.crateLayer;
    return matchesSearch && matchesContext && matchesLayer;
  });
  const visible = new Set(packages.map((item) => item.id));
  const dependencies = (state.crates.dependencies || []).filter((item) => visible.has(item.from) && visible.has(item.to));
  if (!packages.length) {
    container.innerHTML = `<div class="graph-empty"><span>⌁</span><h3>No crates match these filters</h3><p>Try a broader search or clear one of the filters.</p></div>`;
    return;
  }
  state.graph?.destroy();
  state.graph = cytoscape({
    container,
    elements: [
      ...packages.map((item) => ({ data: { id: item.id, label: item.name, layer: item.layer || 'package', context: item.context || '' } })),
      ...dependencies.map((item, index) => ({ data: { id: `edge-${index}-${item.from}-${item.to}`, source: item.from, target: item.to, label: item.name || '' } })),
    ],
    minZoom: 0.25,
    maxZoom: 2.5,
    style: [
      { selector: 'node', style: { 'background-color': '#5ed0a4', label: 'data(label)', color: '#e5f5ef', 'font-size': 11, 'font-family': 'ui-monospace, SFMono-Regular, Menlo, monospace', 'text-wrap': 'ellipsis', 'text-max-width': 130, 'text-valign': 'center', 'text-halign': 'center', width: 38, height: 38, 'border-width': 2, 'border-color': '#18382f', 'overlay-opacity': 0 } },
      { selector: 'node[layer = "domain"]', style: { 'background-color': '#eead69' } },
      { selector: 'node[layer = "adapter"]', style: { 'background-color': '#7f9cf5' } },
      { selector: 'node[layer = "interface"]', style: { 'background-color': '#ce8ee5' } },
      { selector: 'node[layer = "application"]', style: { 'background-color': '#5ed0a4' } },
      { selector: 'node:selected', style: { 'border-color': '#f5c96a', 'border-width': 4 } },
      { selector: 'edge', style: { width: 1, 'line-color': '#30474b', 'target-arrow-color': '#688083', 'target-arrow-shape': 'triangle', 'curve-style': 'bezier', opacity: 0.72 } },
    ],
    layout: { name: packages.length > 45 ? 'grid' : 'cose', animate: false, padding: 40, fit: true, idealEdgeLength: 100, nodeRepulsion: 5500, gravity: 0.35 },
  });
  if (state.selectedCrate && visible.has(state.selectedCrate)) state.graph.$id(state.selectedCrate).select();
  state.graph.on('tap', 'node', (event) => {
    state.selectedCrate = event.target.id();
    state.graph.elements('node').unselect();
    event.target.select();
    const detail = document.querySelector('#crate-details');
    if (detail) detail.innerHTML = renderCrateDetails();
    bindDetailLinks();
  });
}

function renderContextGraph(container) {
  const packages = state.crates.packages || [];
  const dependencies = state.crates.dependencies || [];
  const packageContexts = new Map(packages.map((item) => [item.id, item.context || 'uncategorized']));
  const contextCounts = new Map();
  packages.forEach((item) => {
    const context = item.context || 'uncategorized';
    contextCounts.set(context, (contextCounts.get(context) || 0) + 1);
  });
  const edgeCounts = new Map();
  dependencies.forEach((dependency) => {
    const from = packageContexts.get(dependency.from);
    const to = packageContexts.get(dependency.to);
    if (!from || !to || from === to) return;
    const key = `${from}\u0000${to}`;
    edgeCounts.set(key, (edgeCounts.get(key) || 0) + 1);
  });
  const elements = [...contextCounts.entries()].map(([context, count]) => ({ data: { id: `context-${slug(context)}`, label: `${context}\n${count} ${count === 1 ? 'crate' : 'crates'}`, context } }));
  const edges = [...edgeCounts.entries()].map(([key, count], index) => {
    const [from, to] = key.split('\u0000');
    return { data: { id: `context-edge-${index}`, source: `context-${slug(from)}`, target: `context-${slug(to)}`, count, label: String(count) } };
  });
  state.graph?.destroy();
  state.graph = cytoscape({
    container,
    elements: [...elements, ...edges],
    minZoom: 0.35,
    maxZoom: 2.5,
    style: [
      { selector: 'node', style: { 'background-color': '#27745f', label: 'data(label)', color: '#e8f6f0', 'font-size': 11, 'font-family': 'ui-monospace, SFMono-Regular, Menlo, monospace', 'font-weight': 600, 'text-wrap': 'wrap', 'text-max-width': 130, 'text-valign': 'center', 'text-halign': 'center', width: 70, height: 70, 'border-width': 2, 'border-color': '#5ed0a4', 'overlay-opacity': 0 } },
      { selector: 'node:selected', style: { 'background-color': '#c99c5b', 'border-color': '#f5c96a', 'border-width': 3, color: '#18211e' } },
      { selector: 'edge', style: { width: 'mapData(count, 1, 40, 1, 5)', 'line-color': '#557a70', 'target-arrow-color': '#80aa99', 'target-arrow-shape': 'triangle', 'curve-style': 'bezier', label: 'data(count)', color: '#77938d', 'font-size': 9, 'font-family': 'ui-monospace, SFMono-Regular, Menlo, monospace', 'text-background-color': '#121e21', 'text-background-opacity': 1, 'text-background-padding': 3 } },
    ],
    layout: { name: 'cose', animate: false, padding: 55, idealEdgeLength: 150, nodeRepulsion: 6500, gravity: 0.35, numIter: 500 },
  });
  state.graph.on('tap', 'node', (event) => {
    state.crateContext = event.target.data('context');
    state.selectedCrate = null;
    render();
  });
}

function renderGrpcPage() {
  const grpc = state.grpc;
  const services = grpc.services || [];
  const packages = unique(services.map((item) => item.package));
  const query = state.grpcSearch.toLowerCase().trim();
  const visible = services.filter((service) => {
    const packageMatch = state.grpcPackage === 'all' || service.package === state.grpcPackage;
    const searchMatch = !query || [service.name, service.id, service.package].some((value) => String(value || '').toLowerCase().includes(query));
    return packageMatch && searchMatch;
  });
  const selectedService = findById(services, state.selectedService) || visible[0];
  if (selectedService && state.selectedService !== selectedService.id) state.selectedService = selectedService.id;
  const methods = selectedService?.methods || [];
  const selectedMethod = methods.find((method) => method.id === state.selectedMethod) || methods[0];
  if (selectedMethod && state.selectedMethod !== selectedMethod.id) state.selectedMethod = selectedMethod.id;
  const messageCount = grpc.messages?.length || 0;
  return `<section class="page-heading"><div><p class="eyebrow">PROTOBUF SURFACE</p><h1>gRPC</h1><p class="page-lede">Services, methods, and the message types moving between them.</p></div><div class="stats"><div class="stat"><strong>${services.length}</strong><span>services</span></div><div class="stat"><strong>${messageCount}</strong><span>messages</span></div><div class="stat"><strong>${grpc.files?.length || 0}</strong><span>proto files</span></div></div></section>
    <section class="toolbar" aria-label="gRPC filters"><label class="search-wrap"><span class="sr-only">Search services</span><span class="search-icon">⌕</span><input id="grpc-search" type="search" placeholder="Search services…" value="${esc(state.grpcSearch)}" /></label><label class="select-wrap"><span class="sr-only">Filter by package</span><select id="grpc-package"><option value="all">All packages</option>${packages.map((value) => `<option value="${esc(value)}" ${state.grpcPackage === value ? 'selected' : ''}>${esc(value)}</option>`).join('')}</select></label><span class="toolbar-note">Choose a service to explore its contract</span></section>
    <section class="grpc-layout"><aside class="service-list" aria-label="gRPC services"><div class="list-heading"><span>Services</span><strong>${visible.length}</strong></div>${visible.length ? visible.map((service) => `<button class="service-item ${service.id === selectedService?.id ? 'selected' : ''}" data-service="${esc(service.id)}"><span class="service-symbol">⌁</span><span class="service-copy"><strong>${esc(service.name)}</strong><small>${esc(service.package || 'no package')}</small></span><span class="service-count">${service.methods?.length || 0}</span></button>`).join('') : '<div class="list-empty">No services match this search.</div>'}</aside><div class="grpc-main">${selectedService ? renderServicePanel(selectedService, selectedMethod) : '<div class="panel-empty"><h2>Select a service</h2><p>Choose a service from the list to inspect its methods.</p></div>'}</div></section>`;
}

function renderServicePanel(service, selectedMethod) {
  const methods = service.methods || [];
  const detailMessage = state.selectedMessage ? findById(state.grpc.messages, state.selectedMessage) : null;
  return `<div class="service-header"><div><p class="eyebrow">SERVICE</p><h2>${esc(service.name)}</h2><p class="mono muted">${esc(service.package || 'package unavailable')} · ${esc(service.file || 'file unavailable')}</p></div><span class="service-badge">${service.external ? 'external' : 'workspace'}</span></div><div class="grpc-graph-card"><div class="graph-card-heading"><div><p class="eyebrow">SERVICE CONTRACT</p><span class="graph-caption">One service · ${methods.length} method${methods.length === 1 ? '' : 's'}</span></div><div class="graph-mini-legend"><span><i class="graph-key service"></i>service</span><span><i class="graph-key method"></i>method</span><span><i class="graph-key message"></i>message</span></div></div><div class="grpc-graph-scroll"><div id="grpc-graph" class="grpc-graph-canvas" role="img" aria-label="Interactive graph of the selected service contract"></div></div><div class="mobile-graph-hint">↔ Swipe to explore the service graph</div></div><div class="method-contract"><div class="method-column"><div class="section-label">Methods <span>${methods.length}</span></div>${methods.length ? methods.map((method) => `<button class="method-row ${method.id === selectedMethod?.id ? 'selected' : ''}" data-method="${esc(method.id)}"><span class="method-verb">RPC</span><span class="method-name">${esc(method.name)}</span><span class="stream-badges">${method.clientStreaming ? '<i title="Client streaming">⇢</i>' : ''}${method.serverStreaming ? '<i title="Server streaming">⇠</i>' : ''}</span><span class="row-chevron">›</span></button>`).join('') : '<p class="muted">No methods recorded.</p>'}</div><div class="method-column method-detail">${detailMessage ? renderMessageDetail(detailMessage) : selectedMethod ? renderMethodDetail(selectedMethod) : '<p class="muted">Select a method.</p>'}</div></div>`;
}

function renderGrpcGraph() {
  const container = document.querySelector('#grpc-graph');
  if (!container) return;
  const service = findById(state.grpc.services, state.selectedService);
  if (!service) return;
  state.grpcGraph?.destroy();
  const elements = [{ data: { id: 'service-root', label: service.name, kind: 'service', ref: service.id } }];
  const edges = [];
  const seenMessages = new Set();
  (service.methods || []).forEach((method, index) => {
    const methodId = `method-${index}`;
    elements.push({ data: { id: methodId, label: method.name, kind: 'method', ref: method.id } });
    edges.push({ data: { id: `edge-service-${index}`, source: 'service-root', target: methodId, role: 'dispatch' } });
    [['request', method.input], ['response', method.output]].forEach(([role, typeName]) => {
      const message = messageFor(typeName);
      const messageId = `message-${message?.id || slug(typeName)}`;
      if (!seenMessages.has(messageId)) {
        seenMessages.add(messageId);
        elements.push({ data: { id: messageId, label: message?.name || shortName(typeName), kind: 'message', ref: message?.id || typeName } });
      }
      edges.push({ data: { id: `edge-${index}-${role}`, source: methodId, target: messageId, role } });
    });
  });
  state.grpcGraph = cytoscape({
    container,
    elements: [...elements, ...edges],
    minZoom: 0.55,
    maxZoom: 2,
    style: [
      { selector: 'node', style: { label: 'data(label)', color: '#dce9e7', 'font-size': 10, 'font-family': 'ui-monospace, SFMono-Regular, Menlo, monospace', 'text-wrap': 'ellipsis', 'text-max-width': 150, 'text-valign': 'center', 'text-halign': 'center', 'border-width': 1, 'border-color': '#385451', padding: '9px', 'overlay-opacity': 0 } },
      { selector: 'node[kind = "service"]', style: { shape: 'roundrectangle', 'background-color': '#c99c5b', color: '#18211e', 'font-weight': 700, 'border-color': '#f0c777' } },
      { selector: 'node[kind = "method"]', style: { shape: 'roundrectangle', 'background-color': '#23604d', 'border-color': '#5ed0a4' } },
      { selector: 'node[kind = "message"]', style: { shape: 'rectangle', 'background-color': '#273b65', 'border-color': '#819df0' } },
      { selector: 'node:selected', style: { 'border-color': '#f5c96a', 'border-width': 3 } },
      { selector: 'edge', style: { width: 1.4, 'line-color': '#506866', 'target-arrow-color': '#78908e', 'target-arrow-shape': 'triangle', 'curve-style': 'bezier' } },
      { selector: 'edge[role = "request"]', style: { 'line-color': '#5ed0a4', 'target-arrow-color': '#5ed0a4' } },
      { selector: 'edge[role = "response"]', style: { 'line-color': '#819df0', 'target-arrow-color': '#819df0', 'line-style': 'dashed' } },
      { selector: 'edge[role = "dispatch"]', style: { 'line-color': '#c99c5b', 'target-arrow-color': '#c99c5b' } },
    ],
    layout: { name: 'breadthfirst', directed: true, roots: ['service-root'], spacingFactor: 1.15, padding: 28, animate: false },
  });
  if (state.selectedMethod) state.grpcGraph.nodes().filter((node) => node.data('ref') === state.selectedMethod).select();
  if (state.selectedMessage) state.grpcGraph.nodes().filter((node) => node.data('ref') === state.selectedMessage).select();
  state.grpcGraph.on('tap', 'node', (event) => {
    const node = event.target;
    if (node.data('kind') === 'method') {
      state.selectedMethod = node.data('ref');
      state.selectedMessage = null;
    } else if (node.data('kind') === 'message') {
      state.selectedMessage = node.data('ref');
    }
    syncSelectionHash();
    render();
  });
  const graphScroll = document.querySelector('.grpc-graph-scroll');
  if (graphScroll) {
    requestAnimationFrame(() => {
      const serviceNode = state.grpcGraph.$id('service-root');
      const center = serviceNode.renderedPosition().x;
      graphScroll.scrollLeft = Math.max(0, center - graphScroll.clientWidth / 2);
    });
  }
}

function messageFor(typeName) {
  const normalized = String(typeName || '').replace(/^\./, '');
  return state.grpc.messages?.find((message) => message.id === normalized || message.name === normalized || `${message.package}.${message.name}` === normalized);
}

function typeButton(typeName) {
  const message = messageFor(typeName);
  if (!message) return `<span class="type-name mono">${esc(shortName(typeName))}</span>`;
  return `<button class="type-link mono" data-message="${esc(message.id)}">${esc(shortName(typeName))}</button>`;
}

function renderMethodDetail(method) {
  const request = messageFor(method.input);
  const response = messageFor(method.output);
  return `<div class="detail-label"><span class="method-verb">RPC</span><p class="eyebrow">METHOD CONTRACT</p></div><h3>${esc(method.name)}</h3><p class="method-route mono">${method.clientStreaming ? 'stream ' : ''}${typeButton(method.input)} <span>→</span> ${method.serverStreaming ? 'stream ' : ''}${typeButton(method.output)}</p><div class="io-grid"><div class="io-card"><span class="io-label">REQUEST</span>${request ? renderMessageSummary(request) : `<p class="muted">${esc(shortName(method.input))}</p>`}</div><div class="io-card"><span class="io-label">RESPONSE</span>${response ? renderMessageSummary(response) : `<p class="muted">${esc(shortName(method.output))}</p>`}</div></div>`;
}

function renderMessageSummary(message) {
  return `<button class="message-title" data-message="${esc(message.id)}"><strong>${esc(message.name)}</strong><span>↗</span></button><p class="muted">${message.fields?.length || 0} fields · ${esc(message.package || 'no package')}</p><ul class="field-list">${(message.fields || []).slice(0, 5).map((field) => `<li><span class="field-number">${esc(field.number)}</span><span>${esc(field.name)}</span><span class="field-type mono">${esc(shortName(field.typeName || field.type))}</span></li>`).join('')}${(message.fields?.length || 0) > 5 ? `<li class="more-fields">+ ${(message.fields?.length || 0) - 5} more fields</li>` : ''}</ul>`;
}

function renderMessageDetail(message) {
  return `<div class="message-detail-head"><button class="back-link" id="back-to-method">← method</button><span class="service-badge">MESSAGE</span></div><p class="eyebrow">MESSAGE TYPE</p><h3>${esc(message.name)}</h3><p class="mono muted">${esc(message.package || 'no package')} · ${esc(message.file || 'file unavailable')}</p><div class="message-table-wrap"><table class="message-table"><thead><tr><th>#</th><th>Field</th><th>Type</th><th>Label</th></tr></thead><tbody>${(message.fields || []).map((field) => `<tr><td class="field-number">${esc(field.number)}</td><td>${esc(field.name)}</td><td>${typeButton(field.typeName || field.type)}</td><td class="muted">${esc(field.label || 'optional')}</td></tr>`).join('')}</tbody></table>${!message.fields?.length ? '<p class="muted table-empty">No fields recorded.</p>' : ''}</div>`;
}

function bindPageEvents() {
  document.querySelectorAll('.nav-link').forEach((link) => link.addEventListener('click', () => { state.route = link.hash.includes('grpc') ? 'grpc' : 'crates'; }));
  document.querySelector('#crate-search')?.addEventListener('input', (event) => { state.crateSearch = event.target.value; rerenderAndRestoreFocus('#crate-search', event.target.selectionStart); });
  document.querySelector('#crate-context')?.addEventListener('change', (event) => { state.crateContext = event.target.value; render(); });
  document.querySelector('#crate-layer')?.addEventListener('change', (event) => { state.crateLayer = event.target.value; render(); });
  document.querySelector('#graph-fit')?.addEventListener('click', () => state.graph?.fit(undefined, 40));
  document.querySelector('#graph-zoom-in')?.addEventListener('click', () => state.graph?.zoom({ level: Math.min(state.graph.zoom() * 1.25, 2.5), renderedPosition: { x: state.graph.width() / 2, y: state.graph.height() / 2 } }));
  document.querySelector('#graph-zoom-out')?.addEventListener('click', () => state.graph?.zoom({ level: Math.max(state.graph.zoom() / 1.25, 0.25), renderedPosition: { x: state.graph.width() / 2, y: state.graph.height() / 2 } }));
  document.querySelector('#graph-overview')?.addEventListener('click', () => { state.crateContext = 'all'; state.crateLayer = 'all'; state.crateSearch = ''; state.selectedCrate = null; render(); });
  bindDetailLinks();
  document.querySelector('#grpc-search')?.addEventListener('input', (event) => { state.grpcSearch = event.target.value; rerenderAndRestoreFocus('#grpc-search', event.target.selectionStart); });
  document.querySelector('#grpc-package')?.addEventListener('change', (event) => { state.grpcPackage = event.target.value; render(); });
  document.querySelectorAll('[data-service]').forEach((button) => button.addEventListener('click', () => { state.selectedService = button.dataset.service; state.selectedMethod = null; state.selectedMessage = null; syncSelectionHash(); render(); }));
  document.querySelectorAll('[data-method]').forEach((button) => button.addEventListener('click', () => { state.selectedMethod = button.dataset.method; state.selectedMessage = null; syncSelectionHash(); render(); }));
  document.querySelectorAll('[data-message]').forEach((button) => button.addEventListener('click', () => { state.selectedMessage = button.dataset.message; syncSelectionHash(); render(); }));
  document.querySelector('#back-to-method')?.addEventListener('click', () => { state.selectedMessage = null; syncSelectionHash(); render(); });
}

function rerenderAndRestoreFocus(selector, cursor) {
  render();
  const input = document.querySelector(selector);
  if (input) {
    input.focus();
    input.setSelectionRange(cursor ?? input.value.length, cursor ?? input.value.length);
  }
}

function bindDetailLinks() {
  document.querySelectorAll('.crate-link').forEach((button) => button.addEventListener('click', () => { state.selectedCrate = button.dataset.crate; state.graph?.elements('node').unselect(); state.graph?.$id(state.selectedCrate).select(); const detail = document.querySelector('#crate-details'); if (detail) detail.innerHTML = renderCrateDetails(); bindDetailLinks(); }));
}

window.addEventListener('hashchange', () => { state.route = routeFromHash(); render(); });
state.route = routeFromHash();
loadData();
