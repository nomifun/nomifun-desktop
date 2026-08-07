import { describe, expect, test } from 'bun:test';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { sortKnowledgeBases } from './knowledgeSort';

const base = (name: string, createdAt: number, updatedAt: number, totalSize: number): IKnowledgeBase => ({
  knowledge_base_id: name as IKnowledgeBase['knowledge_base_id'],
  name,
  description: '',
  root_path: '',
  managed: false,
  created_at: createdAt,
  updated_at: updatedAt,
  file_count: 0,
  total_size: totalSize,
  root_exists: true,
  tags: [],
  kind: 'blank',
});

const bases = [
  base('Beta', 10, 30, 20),
  base('Alpha', 30, 20, 10),
  base('Gamma', 20, 10, 30),
];

describe('knowledge list sorting', () => {
  test('applies ascending and descending order to every sort field', () => {
    expect(sortKnowledgeBases(bases, 'updated', 'asc').map((item) => item.name)).toEqual(['Gamma', 'Alpha', 'Beta']);
    expect(sortKnowledgeBases(bases, 'updated', 'desc').map((item) => item.name)).toEqual(['Beta', 'Alpha', 'Gamma']);
    expect(sortKnowledgeBases(bases, 'created', 'asc').map((item) => item.name)).toEqual(['Beta', 'Gamma', 'Alpha']);
    expect(sortKnowledgeBases(bases, 'created', 'desc').map((item) => item.name)).toEqual(['Alpha', 'Gamma', 'Beta']);
    expect(sortKnowledgeBases(bases, 'name', 'asc').map((item) => item.name)).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(sortKnowledgeBases(bases, 'name', 'desc').map((item) => item.name)).toEqual(['Gamma', 'Beta', 'Alpha']);
    expect(sortKnowledgeBases(bases, 'size', 'asc').map((item) => item.name)).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(sortKnowledgeBases(bases, 'size', 'desc').map((item) => item.name)).toEqual(['Gamma', 'Beta', 'Alpha']);
  });

  test('does not mutate the source list', () => {
    const source = [...bases];

    sortKnowledgeBases(source, 'updated', 'desc');

    expect(source.map((item) => item.name)).toEqual(['Beta', 'Alpha', 'Gamma']);
  });
});
