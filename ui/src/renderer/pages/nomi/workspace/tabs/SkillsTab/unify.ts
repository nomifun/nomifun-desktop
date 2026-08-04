/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICompanionSkill, ICompanionSkillConfig } from '@/common/adapter/ipcBridge';

/**
 * One companion owns exactly ONE list of skills. Two very different mechanisms
 * feed it — capabilities granted from the global Skill catalog, and skills the
 * companion mined out of real work — so the merge happens here, in pure code,
 * and the UI only ever renders `SkillEntry`.
 */

/** Shape of a catalog row as returned by `fs.listAvailableSkills`. */
export interface CatalogSkillInfo {
  name: string;
  description: string;
  name_i18n?: Record<string, string>;
  description_i18n?: Record<string, string>;
  location: string;
  source: string;
}

export interface GeneratedSkillEntry {
  kind: 'generated';
  key: string;
  name: string;
  description: string;
  status: ICompanionSkill['status'];
  skill: ICompanionSkill;
}

export interface CatalogSkillEntry {
  kind: 'catalog';
  key: string;
  name: string;
  description: string;
  /** Auto-injected default capability: granted unless explicitly opted out. */
  isAuto: boolean;
  /** False when the config still grants a Skill that is no longer installed. */
  installed: boolean;
  location: string;
  source: string;
}

export type SkillEntry = GeneratedSkillEntry | CatalogSkillEntry;

/** Toolbar filter: everything, only mined skills, only catalog capabilities. */
export type SkillSourceFilter = 'all' | 'generated' | 'catalog';

export const EMPTY_SKILL_CONFIG: ICompanionSkillConfig = { enabled: [], disabled_auto: [] };

/**
 * Effective grant set, mirroring the backend: (auto ∪ enabled) \ disabled_auto.
 * A name may be granted without being installed (the Skill was removed after
 * the companion was configured) — the row then shows an 未安装 marker.
 */
export const isSkillGranted = (
  config: ICompanionSkillConfig,
  autoNames: ReadonlySet<string>,
  name: string
): boolean => !config.disabled_auto.includes(name) && (autoNames.has(name) || config.enabled.includes(name));

export const grantedSkillNames = (
  config: ICompanionSkillConfig,
  autoNames: ReadonlySet<string>,
  catalogNames: readonly string[]
): string[] => {
  const candidates = new Set<string>([...autoNames, ...config.enabled, ...catalogNames]);
  return [...candidates].filter((name) => isSkillGranted(config, autoNames, name));
};

export interface BuildSkillEntriesInput {
  generated: readonly ICompanionSkill[];
  catalog: readonly CatalogSkillInfo[];
  autoNames: ReadonlySet<string>;
  config: ICompanionSkillConfig;
  /** Description used for a granted-but-uninstalled catalog entry. */
  missingDescription: string;
  /** Localized display resolution (name/description i18n maps). */
  display?: (skill: CatalogSkillInfo) => { name: string; description: string };
}

/**
 * Row order encodes urgency, not taxonomy: drafts wait on the user so they come
 * first, then live skills, then granted capabilities, then archived leftovers.
 */
const rankOf = (entry: SkillEntry): number => {
  if (entry.kind === 'generated') {
    if (entry.status === 'draft') return 0;
    return entry.status === 'active' ? 1 : 3;
  }
  return 2;
};

const compareEntries = (a: SkillEntry, b: SkillEntry): number => {
  const rank = rankOf(a) - rankOf(b);
  if (rank !== 0) return rank;
  if (a.kind === 'generated' && b.kind === 'generated') {
    const recency = b.skill.updated_at - a.skill.updated_at;
    if (recency !== 0) return recency;
  }
  if (a.kind === 'catalog' && b.kind === 'catalog' && a.isAuto !== b.isAuto) return a.isAuto ? -1 : 1;
  return a.name.localeCompare(b.name);
};

/** Merge the two sources into the single ordered list the tab renders. */
export const buildSkillEntries = ({
  generated,
  catalog,
  autoNames,
  config,
  missingDescription,
  display,
}: BuildSkillEntriesInput): SkillEntry[] => {
  const byName = new Map(catalog.map((skill) => [skill.name, skill]));
  const entries: SkillEntry[] = generated.map((skill) => ({
    kind: 'generated',
    key: `generated:${skill.companion_skill_id}`,
    name: skill.skill_name,
    description: skill.description,
    status: skill.status,
    skill,
  }));

  for (const name of grantedSkillNames(config, autoNames, [...byName.keys()])) {
    const info = byName.get(name);
    const resolved = info && display ? display(info) : undefined;
    entries.push({
      kind: 'catalog',
      key: `catalog:${name}`,
      name: resolved?.name || name,
      description: info ? resolved?.description || info.description : missingDescription,
      isAuto: autoNames.has(name),
      installed: Boolean(info?.location),
      location: info?.location ?? '',
      source: info?.source ?? 'custom',
    });
  }

  return entries.sort(compareEntries);
};

export const filterSkillEntries = (entries: readonly SkillEntry[], filter: SkillSourceFilter): SkillEntry[] => {
  if (filter === 'all') return [...entries];
  const kind = filter === 'generated' ? 'generated' : 'catalog';
  return entries.filter((entry) => entry.kind === kind);
};

export const countDrafts = (entries: readonly SkillEntry[]): number =>
  entries.filter((entry) => entry.kind === 'generated' && entry.status === 'draft').length;
