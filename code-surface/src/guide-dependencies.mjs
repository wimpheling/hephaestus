import { guideHref } from './navigation.mjs';

/**
 * Return the guide route for a workspace crate. Guides without a crateId are
 * intentionally ignored so conceptual pages never become dependency targets.
 */
export function crateGuideHref(crateId, guides) {
  const guide = (guides || []).find((candidate) => candidate.crateId === crateId);
  return guide ? guideHref(guide) : null;
}

function dependencyAnnotation(records) {
  const kinds = new Set(records.flatMap((record) => record.kinds || []));
  const annotations = [];
  const sortedKinds = [...kinds].sort((left, right) => left.localeCompare(right));
  if (sortedKinds.length > 1) annotations.push(sortedKinds.join(', '));
  else if (sortedKinds[0] && sortedKinds[0] !== 'normal') annotations.push(sortedKinds[0]);
  if (records.length > 0 && records.every((record) => record.optional === true)) annotations.push('optional');

  // A target qualifier describes the whole pair only when every edge is
  // conditional. A required `all` edge alongside a cfg edge must stay
  // unqualified so the merged row does not imply a narrower dependency.
  const conditionalTargets = records.map((record) => (record.targets || []).filter((value) => value !== 'all'));
  if (conditionalTargets.length > 0 && conditionalTargets.every((targets) => targets.length > 0)) {
    for (const target of [...new Set(conditionalTargets.flat())].sort((left, right) => left.localeCompare(right))) {
      annotations.push(target);
    }
  }
  return annotations.join(' · ');
}

function relationEntries(records, packagesById, guides) {
  const grouped = new Map();
  for (const record of records) {
    const crateId = record.crateId;
    const packageRecord = packagesById.get(crateId);
    const guideHref = crateGuideHref(crateId, guides);
    if (!packageRecord || !guideHref) continue;
    const existing = grouped.get(crateId);
    if (existing) {
      existing.records.push(record);
    } else {
      grouped.set(crateId, {
        id: crateId,
        name: packageRecord.name || crateId,
        href: guideHref,
        records: [record],
      });
    }
  }
  return [...grouped.values()]
    .map(({ records: relationRecords, ...entry }) => ({
      ...entry,
      annotation: dependencyAnnotation(relationRecords),
    }))
    .sort((left, right) => left.name.localeCompare(right.name) || left.id.localeCompare(right.id));
}

/**
 * Build the direct local workspace relations for one crate guide.
 *
 * The generated dependency inventory may contain more than one edge for a
 * crate pair (for example when Cargo records different targets or kinds). The
 * guide presents one entry per crate and folds those edge attributes into a
 * compact annotation.
 */
export function crateGuideDependencies(crates, guides, crateId) {
  const packages = crates?.packages || [];
  const dependencies = crates?.dependencies || [];
  const packagesById = new Map(packages.map((packageRecord) => [packageRecord.id, packageRecord]));
  if (!packagesById.has(crateId)) return { dependsOn: [], usedBy: [] };

  return {
    dependsOn: relationEntries(
      dependencies
        .filter((dependency) => dependency.from === crateId)
        .map((dependency) => ({ crateId: dependency.to, ...dependency })),
      packagesById,
      guides,
    ),
    usedBy: relationEntries(
      dependencies
        .filter((dependency) => dependency.to === crateId)
        .map((dependency) => ({ crateId: dependency.from, ...dependency })),
      packagesById,
      guides,
    ),
  };
}
