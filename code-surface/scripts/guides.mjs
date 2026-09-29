import { existsSync, readFileSync, statSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";

// Keep this list small and declarative. A guide is a source README plus the
// generated inventory entries it explains. The generator checks both sides of
// every reference, so moving a context or RPC service leaves an actionable
// error in the generated surface.
export const GUIDE_MANIFEST = [
  {
    id: "root",
    title: "Hephaestus",
    summary: "The repository map and the source of truth for this code surface.",
    path: "README.md",
    contexts: [],
    services: [],
  },
  {
    id: "core",
    parent: "root",
    title: "Heph core",
    summary: "The portable contexts and contracts that make up Heph.",
    path: "crates/heph-core/README.md",
    contexts: [],
    services: [],
  },
  {
    parent: "core",
    id: "auth",
    title: "Authentication and identity",
    summary: "Identity and authorization boundaries.",
    path: "crates/heph-core/auth/README.md",
    contexts: ["authorization", "identity", "secret"],
    services: [
      "hephaestus.identity.v1.IdentityService",
      "hephaestus.pat.v1.PersonalAccessTokenService",
      "hephaestus.secret.v1.SecretService",
    ],
  },
  {
    id: "auth-identity",
    parent: "auth",
    title: "Identity",
    summary: "The identity subcontext and its transport contract.",
    path: "crates/heph-core/auth/identity/README.md",
    contexts: ["identity"],
    services: ["hephaestus.identity.v1.IdentityService"],
  },
  {
    parent: "core",
    id: "forge",
    title: "Forge",
    summary: "Repositories, projects, artifacts, and releases.",
    path: "crates/heph-core/forge/README.md",
    contexts: ["build", "forge", "registry", "release", "review"],
    services: [
      "hephaestus.artifact.v1.ArtifactService",
      "hephaestus.build.v1.BuildService",
      "hephaestus.project.v1.ProjectService",
      "hephaestus.release.v1.ReleaseService",
      "hephaestus.repository.v1.RepositoryService",
      "hephaestus.repository_browser.v1.RepositoryBrowserService",
    ],
  },
  {
    parent: "core",
    id: "image",
    title: "Images",
    summary: "Image catalog and image lifecycle boundaries.",
    path: "crates/heph-core/image/README.md",
    contexts: ["image"],
    services: ["hephaestus.image.v1.ImageCatalogService"],
  },
  {
    parent: "core",
    id: "runtime",
    title: "Runtime",
    summary: "Runs, workspaces, volumes, and runtime providers.",
    path: "crates/heph-core/runtime/README.md",
    contexts: ["mailbox", "run", "runtime", "volume", "vm", "workspace"],
    services: [
      "hephaestus.instance.v1.AgentInstanceService",
      "hephaestus.run.v1.RunService",
    ],
  },
  {
    parent: "core",
    id: "platform",
    title: "Platform",
    summary: "Portable platform types and cross-domain boundary contracts.",
    path: "crates/heph-core/platform/README.md",
    contexts: ["event", "gateway", "platform"],
    services: [
      "hephaestus.event.v1.ProductEventService",
      "hephaestus.gateway.v1.GatewayService",
    ],
  },
  {
    parent: "root",
    id: "std",
    title: "Heph standard providers",
    summary: "Opinionated provider implementations for the portable core.",
    path: "crates/heph-std/README.md",
    contexts: [],
    services: [],
  },
  {
    parent: "root",
    id: "app",
    title: "Heph app",
    summary: "The composition root and distribution entry point.",
    path: "crates/heph-app/README.md",
    optional: true,
    contexts: ["bootstrap"],
    services: [],
  },
];

function encodeRef(ref) {
  return String(ref)
    .split('/')
    .map((segment) => encodeURIComponent(segment))
    .join('/');
}

function githubRepository(remote) {
  const value = String(remote || '').trim().replace(/\.git$/, '');
  const match = /^(?:https?:\/\/github\.com[/:]|git@github\.com:|ssh:\/\/git@github\.com\/)([^/]+)\/([^/]+)$/i.exec(value);
  if (!match) return null;
  return { owner: match[1], name: match[2] };
}

/**
 * Build a source URL for the checked-out repository revision. Branch refs are
 * preferred so links remain useful for an unmerged branch after it is pushed;
 * callers can pass a commit SHA when the checkout has no symbolic branch.
 */
export function sourceBaseForRepository(remote, ref) {
  return sourceBaseForRepositoryKind(remote, ref, "blob");
}

/** Build the branch-aware GitHub tree base used for authored directory links. */
export function sourceTreeBaseForRepository(remote, ref) {
  return sourceBaseForRepositoryKind(remote, ref, "tree");
}

function sourceBaseForRepositoryKind(remote, ref, kind) {
  const repository = githubRepository(remote);
  const reference = String(ref || '').trim();
  if (!repository || !reference) return null;
  return `https://github.com/${repository.owner}/${repository.name}/${kind}/${encodeRef(reference)}/`;
}

function sorted(values) {
  return [...values].sort((left, right) => left.localeCompare(right));
}

function assertSafeGuidePath(repositoryRoot, guidePath) {
  if (isAbsolute(guidePath)) throw new Error(`Guide path must be relative: ${guidePath}`);
  const absolutePath = resolve(repositoryRoot, guidePath);
  const escaped = relative(repositoryRoot, absolutePath).startsWith("..");
  if (escaped) throw new Error(`Guide path escapes the repository: ${guidePath}`);
  if (!guidePath.endsWith("/README.md") && guidePath !== "README.md") {
    throw new Error(`Guide path must point to a README.md: ${guidePath}`);
  }
  return absolutePath;
}

function assertUnique(values, label) {
  const seen = new Set();
  for (const value of values) {
    if (seen.has(value)) throw new Error(`Duplicate ${label}: ${value}`);
    seen.add(value);
  }
}

function normalizedGuideLinkPath(destination, sourcePath) {
  const value = String(destination || '').trim();
  if (!value || value.startsWith('#') || /^(?:https?:\/\/|mailto:|tel:)/i.test(value)) return null;
  if (/^[a-z][a-z\d+.-]*:/i.test(value) || value.startsWith('//')) return null;
  const path = value.search(/[?#]/) < 0 ? value : value.slice(0, value.search(/[?#]/));
  const segments = path.startsWith('/') ? [] : String(sourcePath).split('/').slice(0, -1);
  for (const segment of path.replace(/^\/+/, '').split('/')) {
    if (!segment || segment === '.') continue;
    if (segment === '..') {
      if (!segments.length) return null;
      segments.pop();
    } else {
      segments.push(segment);
    }
  }
  return segments.join('/');
}

function authoredSourceDirectories(repositoryRoot, guides) {
  const directories = new Set();
  const linkPattern = /\]\(([^\s)]+)(?:\s+[^)]*)?\)/g;
  for (const guide of guides) {
    for (const match of guide.content.matchAll(linkPattern)) {
      const path = normalizedGuideLinkPath(match[1], guide.path);
      if (!path) continue;
      try {
        if (statSync(resolve(repositoryRoot, path)).isDirectory()) directories.add(path);
      } catch {
        // Broken links remain source links; generation does not turn this
        // inventory into a second repository-wide link checker.
      }
    }
  }
  return sorted([...directories]);
}

/**
 * Resolve the static manifest against generated Cargo and Buf inventories.
 * Missing optional guides are omitted; missing required guides fail generation
 * so a renamed source cannot silently disappear from the UI.
 */
export function generateGuides({ repositoryRoot, crates, grpc, sourceBase = null, sourceTreeBase = null }) {
  const contexts = new Set((crates.packages ?? []).map((pkg) => pkg.context).filter(Boolean));
  const services = new Set((grpc.services ?? []).map((service) => service.id));
  assertUnique(GUIDE_MANIFEST.map((guide) => guide.id), "guide ID");
  assertUnique(GUIDE_MANIFEST.map((guide) => guide.path), "guide path");

  const errors = [];
  const guides = [];
  for (const entry of GUIDE_MANIFEST) {
    const absolutePath = assertSafeGuidePath(repositoryRoot, entry.path);
    for (const context of entry.contexts ?? []) {
      if (!contexts.has(context)) errors.push(`${entry.id} references unknown Cargo context ${context}`);
    }
    for (const service of entry.services ?? []) {
      if (!services.has(service)) errors.push(`${entry.id} references unknown gRPC service ${service}`);
    }
    if (!existsSync(absolutePath)) {
      if (entry.optional) continue;
      errors.push(`${entry.id} references missing guide source ${entry.path}`);
      continue;
    }
    guides.push({
      id: entry.id,
      title: entry.title,
      summary: entry.summary,
      path: entry.path,
      parent: entry.parent ?? null,
      contexts: sorted(entry.contexts ?? []),
      services: sorted(entry.services ?? []),
      content: readFileSync(absolutePath, "utf8"),
    });
  }

  if (errors.length > 0) throw new Error(`Guide manifest validation failed:\n${errors.map((error) => `- ${error}`).join("\n")}`);

  const placedContexts = new Set(GUIDE_MANIFEST.flatMap((guide) => guide.contexts ?? []));
  const placedServices = new Set(GUIDE_MANIFEST.flatMap((guide) => guide.services ?? []));
  const sourceDirectories = authoredSourceDirectories(repositoryRoot, guides);
  return {
    schemaVersion: 1,
    sourceBase,
    sourceTreeBase,
    sourceDirectories,
    guides,
    unplaced: {
      contexts: sorted([...contexts].filter((context) => !placedContexts.has(context))),
      services: sorted([...services].filter((service) => !placedServices.has(service))),
    },
  };
}
