/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type KnowledgeBaseSearchItem = {
  kind: string;
  name: string;
  tags: string[];
};

export type KnowledgeBaseLabelMap = Record<string, string | undefined>;

const normalizeSearchText = (value: string): string => value.trim().toLocaleLowerCase();

export const shouldShowKnowledgeBaseSearch = (baseCount: number): boolean => baseCount > 1;

export function filterKnowledgeBasesByQuery<T extends KnowledgeBaseSearchItem>(
  bases: T[],
  query: string,
  tagLabelsByKey: KnowledgeBaseLabelMap,
  kindLabelsByKind: KnowledgeBaseLabelMap
): T[] {
  const normalizedQuery = normalizeSearchText(query);
  if (!normalizedQuery) return bases;

  return bases.filter((base) => {
    const haystacks = [
      base.name,
      kindLabelsByKind[base.kind],
      ...base.tags.map((tagKey) => tagLabelsByKey[tagKey]),
    ];

    return haystacks.some((text) => normalizeSearchText(text ?? '').includes(normalizedQuery));
  });
}
