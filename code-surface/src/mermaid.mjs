let mermaidModule;
let diagramNumber = 0;

function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

export function hasMermaidFence(markdown) {
  return /^```mermaid\s*$/m.test(String(markdown || ''));
}

export function mermaidBlockMarkup(source) {
  return `<div class="mermaid-diagram"><pre class="mermaid-source"><code>${escapeHtml(source)}</code></pre></div>`;
}

async function loadMermaid() {
  mermaidModule ??= import('mermaid').then((module) => module.default || module);
  return mermaidModule;
}

export async function renderMermaidDiagrams(root) {
  const blocks = [...root.querySelectorAll('.mermaid-diagram:not([data-mermaid-rendered])')];
  if (!blocks.length) return;
  let mermaid;
  try {
    mermaid = await loadMermaid();
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });
  } catch (_error) {
    markFailed(blocks);
    return;
  }
  for (const block of blocks) {
    const source = block.querySelector('.mermaid-source')?.textContent || '';
    block.dataset.mermaidRendered = 'true';
    try {
      const result = await mermaid.render(`code-surface-mermaid-${diagramNumber += 1}`, source);
      block.innerHTML = result.svg;
      const viewBox = block.querySelector('svg')?.getAttribute('viewBox')?.split(/\s+/).map(Number);
      if (viewBox?.length === 4 && Number.isFinite(viewBox[2]) && viewBox[2] > 0) {
        block.style.setProperty('--mermaid-natural-width', `${Math.ceil(viewBox[2])}px`);
      }
      result.bindFunctions?.(block);
    } catch (_error) {
      markFailed([block]);
    }
  }
}

function markFailed(blocks) {
  for (const block of blocks) {
    block.dataset.mermaidRendered = 'true';
    block.classList.add('mermaid-failed');
    const notice = block.ownerDocument.createElement('p');
    notice.className = 'mermaid-error';
    notice.setAttribute('role', 'status');
    notice.textContent = 'Diagram unavailable; source shown.';
    block.append(notice);
  }
}
