import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

// rustdoc's all.html is deliberately used as the input format here. It is a
// stable, generated view of the public API and avoids guessing visibility from
// Rust source text. The parser only depends on the section headings and links
// rustdoc emits for each public item.
const KIND_NAMES = new Map([
  ["modules", "module"],
  ["structs", "struct"],
  ["enums", "enum"],
  ["traits", "trait"],
  ["functions", "function"],
  ["type-aliases", "type"],
  ["types", "type"],
  ["constants", "constant"],
  ["statics", "static"],
  ["macros", "macro"],
  ["unions", "union"],
  ["keywords", "keyword"],
]);

function decodeHtml(value) {
  return String(value)
    .replace(/<[^>]*>/g, "")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;|&apos;/g, "'")
    .replace(/&#x([0-9a-f]+);/gi, (_match, code) => String.fromCodePoint(Number.parseInt(code, 16)))
    .replace(/&#(\d+);/g, (_match, code) => String.fromCodePoint(Number.parseInt(code, 10)))
    .replace(/\s+/g, " ")
    .trim();
}

function normalizeHref(href) {
  const value = decodeHtml(href);
  if (!value || value.startsWith("#") || /^[a-z][a-z\d+.-]*:/i.test(value) || value.startsWith("//")) return null;
  if (value.includes("\\") || value.split("/").includes("..")) return null;
  return value;
}

function sectionKind(id, title) {
  const normalizedId = String(id || "").toLowerCase();
  if (KIND_NAMES.has(normalizedId)) return KIND_NAMES.get(normalizedId);
  const normalizedTitle = String(title || "").toLowerCase().replace(/\s+/g, "-");
  return KIND_NAMES.get(normalizedTitle) ?? null;
}

function parseSections(html) {
  const headings = [...String(html).matchAll(/<h[23]\b[^>]*\bid\s*=\s*(["'])(.*?)\1[^>]*>([\s\S]*?)<\/h[23]>/gi)];
  return headings.map((heading, index) => ({
    id: heading[2],
    title: decodeHtml(heading[3]),
    body: String(html).slice(heading.index + heading[0].length, headings[index + 1]?.index ?? String(html).length),
  }));
}

/** Parse the public items listed by one rustdoc `all.html` page. */
export function parseRustdocAllHtml(html, { baseHref = "" } = {}) {
  const items = [];
  for (const section of parseSections(html)) {
    const kind = sectionKind(section.id, section.title);
    if (!kind) continue;
    const list = section.body.match(/<ul\b[^>]*class\s*=\s*(["'])[^"']*\ball-items\b[^"']*\1[^>]*>([\s\S]*?)<\/ul>/i);
    if (!list) continue;
    const links = [...list[2].matchAll(/<a\b[^>]*href\s*=\s*(["'])(.*?)\1[^>]*>([\s\S]*?)<\/a>/gi)];
    for (const link of links) {
      const href = normalizeHref(link[2]);
      const name = decodeHtml(link[3]);
      if (!href || !name) continue;
      items.push({ kind, name, href: `${baseHref}${href}` });
    }
  }
  const unique = new Map(items.map((item) => [`${item.kind}\0${item.name}\0${item.href}`, item]));
  return [...unique.values()].sort((left, right) =>
    left.kind.localeCompare(right.kind) || left.name.localeCompare(right.name) || left.href.localeCompare(right.href),
  );
}

/**
 * Top-level modules are listed on rustdoc's crate index rather than all.html
 * in current rustdoc releases. Keep this small companion parser so modules
 * remain visible while the all-items page remains the canonical item source.
 */
export function parseRustdocModulesHtml(html, { baseHref = "" } = {}) {
  const items = [];
  for (const section of parseSections(html)) {
    if (String(section.id).toLowerCase() !== "modules") continue;
    const list = section.body.match(/<dl\b[^>]*class\s*=\s*(["'])[^"']*\bitem-table\b[^"']*\1[^>]*>([\s\S]*?)<\/dl>/i);
    if (!list) continue;
    for (const link of list[2].matchAll(/<a\b([^>]*)>([\s\S]*?)<\/a>/gi)) {
      const classMatch = link[1].match(/\bclass\s*=\s*(["'])(.*?)\1/i);
      const hrefMatch = link[1].match(/\bhref\s*=\s*(["'])(.*?)\1/i);
      if (!classMatch?.[2].split(/\s+/).includes("mod") || !hrefMatch) continue;
      const href = normalizeHref(hrefMatch[2]);
      const name = decodeHtml(link[2]);
      if (href && name) items.push({ kind: "module", name, href: `${baseHref}${href}` });
    }
  }
  return [...new Map(items.map((item) => [item.href, item])).values()].sort((left, right) => left.name.localeCompare(right.name) || left.href.localeCompare(right.href));
}

function rustdocCrateDirectory(targetName) {
  // Cargo/rustdoc use underscores for crate directory names, including when
  // the package and library target use their conventional hyphenated names.
  return String(targetName).replaceAll("-", "_");
}

function libraryTarget(packageRecord) {
  return (packageRecord.targets ?? [])
    .filter((target) => (target.kind ?? []).includes("lib"))
    .sort((left, right) => left.name.localeCompare(right.name))[0] ?? null;
}

/**
 * Read rustdoc output for workspace packages. Missing docs remain explicit so
 * standalone builds can render a useful state when Cargo is unavailable.
 */
export function generateRustApi(packages, { docRoot, read = readFileSync, docsReady = true } = {}) {
  return packages.map((packageRecord) => {
    const target = libraryTarget(packageRecord);
    if (!target) {
      return { status: "no-library-target", items: [] };
    }
    const directory = rustdocCrateDirectory(target.name);
    const allPath = resolve(docRoot, directory, "all.html");
    const indexPath = resolve(docRoot, directory, "index.html");
    if (!docsReady || !existsSync(allPath)) {
      return { status: "unavailable", items: [] };
    }
    try {
      const modules = existsSync(indexPath) ? parseRustdocModulesHtml(read(indexPath, "utf8"), { baseHref: `target/doc/${directory}/` }) : [];
      return {
        status: "available",
        target: target.name,
        path: `target/doc/${directory}/all.html`,
        items: [...new Map([...parseRustdocAllHtml(read(allPath, "utf8"), { baseHref: `target/doc/${directory}/` }), ...modules].map((item) => [`${item.kind}\0${item.name}\0${item.href}`, item])).values()].sort((left, right) => left.kind.localeCompare(right.kind) || left.name.localeCompare(right.name) || left.href.localeCompare(right.href)),
      };
    } catch {
      return { status: "unavailable", items: [] };
    }
  });
}

export { libraryTarget, rustdocCrateDirectory };
