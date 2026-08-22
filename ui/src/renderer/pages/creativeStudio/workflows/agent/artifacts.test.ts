/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { CreativeStudioContractError } from '../../domain';

import {
  CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
  MAX_CREATIVE_WORKFLOW_DRAFT_JSON_BYTES,
  parseCreativeWorkflowDraftArtifact,
  type CreativeWorkflowDraftArtifact,
} from './artifacts';

const artifact = (
  overrides: Partial<CreativeWorkflowDraftArtifact['draft']> = {}
): CreativeWorkflowDraftArtifact => ({
  kind: CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
  summary: '生成一个可直接编辑的工作流草稿',
  draft: {
    mode: 'single-image',
    name: '电商主图',
    description: '根据产品信息生成一张主图。',
    category: '电商',
    promptTemplate: '为 {{product_name}} 生成主图，突出 {{selling_points}}。',
    ...overrides,
  },
});

const fence = (value: unknown): string =>
  '```json\n' + JSON.stringify(value) + '\n```';

const expectFailure = (text: string): CreativeStudioContractError => {
  try {
    parseCreativeWorkflowDraftArtifact(text);
    throw new Error('Expected workflow draft artifact contract failure');
  } catch (error) {
    expect(error instanceof CreativeStudioContractError).toBe(true);
    return error as CreativeStudioContractError;
  }
};

describe('Creative Workflow Agent artifact parser', () => {
  test('accepts one unique final lowercase json fence after conversational prose', () => {
    const expected = artifact();
    expect(
      parseCreativeWorkflowDraftArtifact(
        `我已把需求收敛为一个最小工作流：\n${fence(expected)}\n \t`
      )
    ).toEqual(expected);
  });

  test('leaves ordinary prose and artifacts owned by other products untouched', () => {
    expect(parseCreativeWorkflowDraftArtifact('先从一个简单工作流开始。')).toBeNull();
    expect(
      parseCreativeWorkflowDraftArtifact(
        fence({ kind: 'nomifun.other/v1', summary: 'other', draft: {} })
      )
    ).toBeNull();
  });

  test('rejects non-lowercase, non-final and non-unique target fences', () => {
    expectFailure('```JSON\n' + JSON.stringify(artifact()) + '\n```');
    expectFailure(`${fence(artifact())}\n不能再追加说明`);
    expectFailure('```text\n说明\n```\n' + fence(artifact()));
  });

  test('enforces exact top-level and draft fields without model-owned configuration', () => {
    expectFailure(fence({ ...artifact(), id: 'model-owned-id' }));
    expectFailure(fence({ ...artifact(), visibility: 'public' }));
    expectFailure(
      fence({
        ...artifact(),
        draft: { ...artifact().draft, tags: ['营销'] },
      })
    );
    expectFailure(
      fence({
        ...artifact(),
        draft: { ...artifact().draft, model: { providerId: 'x', model: 'y' } },
      })
    );
    expectFailure(
      fence({
        ...artifact(),
        draft: { ...artifact().draft, variables: [] },
      })
    );
  });

  test('rejects duplicate decoded keys at every object level', () => {
    expectFailure(
      '```json\n{"kind":"' +
        CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND +
        '","summary":"first","\\u0073ummary":"second","draft":' +
        JSON.stringify(artifact().draft) +
        '}\n```'
    );
    expectFailure(
      '```json\n{"kind":"' +
        CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND +
        '","summary":"draft","draft":{"mode":"single-image","name":"first","\\u006eame":"second","description":"","category":"","promptTemplate":"make {{product_name}}"}}\n```'
    );
  });

  test('enforces the two modes and trimmed bounded strings', () => {
    expectFailure(fence(artifact({ mode: 'video' as 'single-image' })));
    expectFailure(fence({ ...artifact(), summary: ' padded ' }));
    expectFailure(fence({ ...artifact(), summary: 'x'.repeat(501) }));
    expectFailure(fence(artifact({ name: '' })));
    expectFailure(fence(artifact({ name: 'x'.repeat(121) })));
    expectFailure(fence(artifact({ description: ' padded ' })));
    expectFailure(fence(artifact({ category: 'x'.repeat(81) })));
    expectFailure(fence(artifact({ promptTemplate: '' })));
  });

  test('rejects unpaired UTF-16 surrogates and oversized JSON', () => {
    expectFailure(fence({ ...artifact(), summary: `bad\ud800summary` }));
    expectFailure(fence(artifact({ promptTemplate: `bad\udc00prompt` })));

    const oversized =
      '```json\n{"kind":"' +
      CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND +
      '","summary":"draft","draft":' +
      JSON.stringify(artifact().draft) +
      ',"padding":"' +
      'x'.repeat(MAX_CREATIVE_WORKFLOW_DRAFT_JSON_BYTES) +
      '"}\n```';
    const error = expectFailure(oversized);
    expect(error.expected.includes('UTF-8 bytes')).toBe(true);
  });
});
