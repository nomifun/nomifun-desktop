/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type ExternalAgentSkill = {
  name: string;
  description: string;
  path: string;
};

export type ExternalAgentSkillSource = {
  name: string;
  path: string;
  source: string;
  skill_count?: number;
  skills: ExternalAgentSkill[];
};

export type AgentSkillImportRow = ExternalAgentSkill & {
  key: string;
  source: string;
  sourceName: string;
  sourcePath: string;
  alreadyImported: boolean;
};

export type AgentSkillImportSummary = {
  selectedCount: number;
  alreadyImportedCount: number;
  importableCount: number;
};

type ImportedSkillForCustomList = {
  name: string;
  alreadyImported: boolean;
};

const rowKey = (source: string, skill: ExternalAgentSkill) => `${source}::${skill.name}::${skill.path}`;

export const buildAgentSkillRows = (
  sources: ExternalAgentSkillSource[],
  existingSkillNames: Set<string>
): AgentSkillImportRow[] =>
  sources.flatMap((source) =>
    source.skills.map((skill) => ({
      ...skill,
      key: rowKey(source.source, skill),
      source: source.source,
      sourceName: source.name,
      sourcePath: source.path,
      alreadyImported: existingSkillNames.has(skill.name),
    }))
  );

export const defaultSelectedAgentSkillKeys = (rows: AgentSkillImportRow[]): string[] =>
  rows.filter((row) => !row.alreadyImported).map((row) => row.key);

export const mergeImportedSkillNames = (current: string[], imported: string[]): string[] =>
  Array.from(new Set([...current, ...imported.filter((name) => name.trim().length > 0)]));

export const customSkillNamesForImportedAgentSkills = (
  imported: ImportedSkillForCustomList[],
  existingCustomSkillNames: Set<string>
): string[] =>
  imported
    .filter((skill) => !skill.alreadyImported || existingCustomSkillNames.has(skill.name))
    .map((skill) => skill.name);

export const summarizeAgentSkillImport = (
  _rows: AgentSkillImportRow[],
  selectedRows: AgentSkillImportRow[]
): AgentSkillImportSummary => {
  const alreadyImportedCount = selectedRows.filter((row) => row.alreadyImported).length;
  return {
    selectedCount: selectedRows.length,
    alreadyImportedCount,
    importableCount: selectedRows.length - alreadyImportedCount,
  };
};
