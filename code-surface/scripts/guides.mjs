import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";

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
    id: "dev",
    title: "Heph development",
    summary: "Development-only tools and provider conformance suites.",
    path: "crates/heph-dev/README.md",
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
  if (isAbsolute(guidePath) || /^[A-Za-z]:[\\/]/.test(guidePath)) throw new Error(`Guide path must be relative: ${guidePath}`);
  const absolutePath = resolve(repositoryRoot, guidePath);
  const escaped = relative(repositoryRoot, absolutePath).startsWith("..");
  if (escaped) throw new Error(`Guide path escapes the repository: ${guidePath}`);
  if (!guidePath.endsWith("/README.md") && guidePath !== "README.md") {
    throw new Error(`Guide path must point to a README.md: ${guidePath}`);
  }
  return absolutePath;
}

function assertSafeRepositoryPath(repositoryRoot, sourcePath, expectedFilename) {
  if (isAbsolute(sourcePath) || /^[A-Za-z]:[\\/]/.test(sourcePath)) throw new Error(`Source path must be relative: ${sourcePath}`);
  const absolutePath = resolve(repositoryRoot, sourcePath);
  const escaped = relative(repositoryRoot, absolutePath).startsWith("..");
  if (escaped) throw new Error(`Source path escapes the repository: ${sourcePath}`);
  if (expectedFilename && !sourcePath.endsWith(`/${expectedFilename}`) && sourcePath !== expectedFilename) {
    throw new Error(`Source path must point to ${expectedFilename}: ${sourcePath}`);
  }
  return absolutePath;
}

function directoryForManifest(manifest) {
  const directory = dirname(String(manifest).replaceAll("\\", "/")).replaceAll("\\", "/");
  return directory === "." ? "" : directory;
}

function directoryId(directory) {
  if (!directory) return "root";
  // Hex keeps generated route IDs stable and safe for hash navigation even
  // when a future directory name contains punctuation or spaces.
  return `directory-${Buffer.from(directory).toString("hex")}`;
}

function directoryTitle(directory) {
  return directory ? directory.split("/").at(-1) : "Hephaestus";
}

function directoryParent(directory) {
  if (!directory) return null;
  const parent = dirname(directory);
  return parent === "." ? "" : parent;
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
 * Resolve authored conceptual guides and the directory tree implied by Cargo
 * metadata. Authored entries own titles, summaries, context associations, and
 * RPC associations; every directory containing a workspace crate is then
 * added beneath them, including directories without a README.
 */
export function generateGuides({ repositoryRoot, crates, grpc, sourceBase = null, sourceTreeBase = null }) {
  const contexts = new Set((crates.packages ?? []).map((pkg) => pkg.context).filter(Boolean));
  const services = new Set((grpc.services ?? []).map((service) => service.id));
  const entries = GUIDE_MANIFEST;
  assertUnique(entries.map((guide) => guide.id), "guide ID");
  assertUnique(entries.map((guide) => guide.path), "guide path");

  const errors = [];
  const authoredByDirectory = new Map();
  for (const entry of entries) {
    const absolutePath = assertSafeGuidePath(repositoryRoot, entry.path);
    const directory = directoryForManifest(entry.path);
    if (authoredByDirectory.has(directory)) {
      errors.push(`Multiple authored guides reference directory ${directory || "."}`);
    }
    authoredByDirectory.set(directory, entry);
    for (const context of entry.contexts ?? []) {
      if (!contexts.has(context)) errors.push(`${entry.id} references unknown Cargo context ${context}`);
    }
    for (const service of entry.services ?? []) {
      if (!services.has(service)) errors.push(`${entry.id} references unknown gRPC service ${service}`);
    }
    // Missing READMEs are represented as navigable guide nodes with an
    // explicit warning flag. The path itself is still validated above, so a
    // typo cannot escape the repository or become a non-README source.
  }

  const packageByDirectory = new Map();
  const directories = new Set([""]);
  for (const pkg of crates.packages ?? []) {
    const manifest = String(pkg.manifest || "").replaceAll("\\", "/");
    let absoluteManifest;
    try {
      absoluteManifest = assertSafeRepositoryPath(repositoryRoot, manifest, "Cargo.toml");
    } catch (error) {
      errors.push(`${pkg.id} has an invalid manifest path: ${error.message}`);
      continue;
    }
    if (!existsSync(absoluteManifest)) {
      errors.push(`${pkg.id} references missing Cargo manifest ${manifest}`);
      continue;
    }
    const directory = directoryForManifest(manifest);
    if (packageByDirectory.has(directory)) {
      errors.push(`Multiple Cargo crates share directory ${directory}`);
    }
    packageByDirectory.set(directory, pkg);
    let current = directory;
    while (true) {
      directories.add(current);
      if (!current) break;
      current = directoryParent(current);
    }
  }

  // Authored conceptual entries can cover a directory with no Cargo manifest,
  // so retain their ancestors in the same navigable tree as crate directories.
  for (const directory of authoredByDirectory.keys()) {
    let current = directory;
    while (true) {
      directories.add(current);
      if (!current) break;
      current = directoryParent(current);
    }
  }

  if (errors.length > 0) throw new Error(`Guide manifest validation failed:\n${errors.map((error) => `- ${error}`).join("\n")}`);

  // `crates/` is the repository's source layout wrapper, rather than a
  // conceptual branch. Keep every directory beneath it while attaching those
  // directories directly to their authored top-level guide.
  const guideDirectories = [...directories].filter((directory) => directory !== "crates" || authoredByDirectory.has(directory));
  const guides = guideDirectories.map((directory) => {
    const authored = authoredByDirectory.get(directory);
    const pkg = packageByDirectory.get(directory);
    const readmePath = directory ? `${directory}/README.md` : "README.md";
    const absoluteReadme = assertSafeGuidePath(repositoryRoot, readmePath);
    const missingDocumentation = !existsSync(absoluteReadme);
    const content = missingDocumentation ? "" : readFileSync(absoluteReadme, "utf8");
    const parentDirectory = directoryParent(directory);
    const parentAuthored = parentDirectory === null ? null : authoredByDirectory.get(parentDirectory);
    const parent = authored?.parent ?? parentAuthored?.id ?? (parentDirectory === null ? null : directoryId(parentDirectory));
    return {
      id: authored?.id ?? directoryId(directory),
      title: authored?.title ?? pkg?.name ?? directoryTitle(directory),
      summary: authored?.summary ?? (pkg?.description || (directory ? `Source directory ${directory}.` : "The repository map and the source of truth for this code surface.")),
      path: readmePath,
      directory,
      parent,
      crateId: pkg?.id ?? null,
      manifest: pkg?.manifest ?? null,
      missingDocumentation,
      contexts: sorted(authored?.contexts ?? []),
      services: sorted(authored?.services ?? []),
      content,
    };
  }).sort((left, right) => left.path.localeCompare(right.path));

  assertUnique(guides.map((guide) => guide.id), "generated guide ID");
  assertUnique(guides.map((guide) => guide.path), "generated guide path");
  const guideById = new Set(guides.map((guide) => guide.id));
  for (const guide of guides) {
    if (guide.parent && !guideById.has(guide.parent)) errors.push(`${guide.id} has an unknown parent ${guide.parent}`);
  }
  const crateGuideCounts = new Map(guides.filter((guide) => guide.crateId).map((guide) => [guide.crateId, 0]));
  for (const guide of guides) {
    if (guide.crateId) crateGuideCounts.set(guide.crateId, (crateGuideCounts.get(guide.crateId) ?? 0) + 1);
  }
  for (const pkg of crates.packages ?? []) {
    if (crateGuideCounts.get(pkg.id) !== 1) errors.push(`${pkg.id} must be represented by exactly one guide directory`);
  }
  if (errors.length > 0) throw new Error(`Guide manifest validation failed:\n${errors.map((error) => `- ${error}`).join("\n")}`);

  const placedContexts = new Set(entries.flatMap((guide) => guide.contexts ?? []));
  const placedServices = new Set(entries.flatMap((guide) => guide.services ?? []));
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
