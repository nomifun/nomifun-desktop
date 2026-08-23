/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { CreativeModelCatalogSnapshot, CreativeModelOption } from '../../models';
import { validateWorkflowDefinition } from '../domain';
import { CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND } from '../agent/artifacts';
import type { WorkflowDraftPort } from '../agent/draftPort';
import {
  WorkflowAgentDraftPreview,
  generateWorkflowAgentDraft,
} from './WorkflowAgentDraftModal';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000951' as ProviderId;
const provider: IProvider = {
  id: PROVIDER_ID,
  platform: 'openai',
  name: 'QA Chat',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: true,
  enabled: true,
  models: [
    {
      provider_id: PROVIDER_ID,
      model: 'nomi-chat',
      enabled: true,
      sort_order: 0,
      capabilities: [
        {
          task: 'chat',
          traits: [],
          protocol: 'openai.chat_text',
          connection_role: 'default',
          allow_cross_origin_credentials: false,
          provider_params: {},
          created_at: 1,
          updated_at: 1,
        },
      ],
      created_at: 1,
      updated_at: 1,
    },
  ],
};
const catalog: CreativeModelCatalogSnapshot = {
  status: 'ready',
  providers: [provider],
  error: null,
};
const model: CreativeModelOption = {
  providerId: PROVIDER_ID,
  providerName: 'QA Chat',
  platform: 'openai',
  model: 'nomi-chat',
  task: 'chat',
  traits: [],
  protocol: 'openai.chat_text',
};
const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

const rejectionMessage = async (promise: Promise<unknown>): Promise<string> => {
  try {
    await promise;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  throw new Error('Expected promise to reject.');
};

describe('minimal Workflow Agent draft modal model', () => {
  test('uses the selected catalog row, parses a strict artifact, and does not persist it', async () => {
    const calls: unknown[] = [];
    const port: WorkflowDraftPort = {
      async draft(input) {
        calls.push(input);
        return {
          text: `\`\`\`json\n${JSON.stringify({
            kind: CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
            summary: '生成单图草稿',
            draft: {
              mode: 'single-image',
              name: '新品主图',
              description: '固定风格的新品海报。',
              category: '电商',
              promptTemplate: '为 {{product_name}} 生成主图，突出 {{selling_points}}。',
            },
          })}\n\`\`\``,
        };
      },
    };

    const generated = await generateWorkflowAgentDraft({
      prompt: '  创建一个新品海报流程  ',
      model,
      catalog,
      port,
    });

    expect(calls).toEqual([
      { providerId: PROVIDER_ID, model: 'nomi-chat', prompt: '创建一个新品海报流程' },
    ]);
    expect(generated.workflow.metadata.name).toBe('新品主图');
    expect(generated.workflow.metadata.visibility).toBe('private');
    expect(validateWorkflowDefinition(generated.workflow).ok).toBe(true);
  });

  test('renders empty and ready previews with explicit manual-save copy', async () => {
    const empty = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <WorkflowAgentDraftPreview draft={null} />
      </I18nextProvider>
    );
    expect(empty.includes('data-workflow-agent-preview="empty"')).toBe(true);
    expect(empty.includes('will not be saved or run automatically')).toBe(true);

    const port: WorkflowDraftPort = {
      async draft() {
        return {
          text: `\`\`\`json\n${JSON.stringify({
            kind: CREATIVE_WORKFLOW_DRAFT_ARTIFACT_KIND,
            summary: 'draft',
            draft: {
              mode: 'multi-image-series',
              name: '社媒多图',
              description: '一组连续配图',
              category: '内容',
              promptTemplate: '围绕 {{topic}}，保持 {{style}}，适配 {{platform}}。',
            },
          })}\n\`\`\``,
        };
      },
    };
    const draft = await generateWorkflowAgentDraft({ prompt: '多图', model, catalog, port });
    const ready = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <WorkflowAgentDraftPreview draft={draft} />
      </I18nextProvider>
    );
    expect(ready.includes('data-workflow-agent-preview="ready"')).toBe(true);
    expect(ready.includes('社媒多图')).toBe(true);
    expect(ready.includes('Review and save the draft manually')).toBe(true);
  });

  test('fails closed when the Provider or final artifact is unavailable', async () => {
    const port: WorkflowDraftPort = {
      async draft() {
        return { text: '只有建议文本' };
      },
    };
    expect((
      await rejectionMessage(
        generateWorkflowAgentDraft({ prompt: '草稿', model, catalog, port })
      )
    ).includes('did not return an applicable')).toBe(true);
    expect((
      await rejectionMessage(generateWorkflowAgentDraft({
        prompt: '草稿',
        model,
        catalog: { ...catalog, providers: [] },
        port,
      }))
    ).includes('selected model is unavailable')).toBe(true);
  });

  test('does not call the backend while a stale catalog is loading or failed', async () => {
    let calls = 0;
    const port: WorkflowDraftPort = {
      async draft() {
        calls += 1;
        return { text: 'unused' };
      },
    };

    for (const status of ['loading', 'error'] as const) {
      const message = await rejectionMessage(
        generateWorkflowAgentDraft({
          prompt: '草稿',
          model,
          catalog: {
            status,
            providers: [provider],
            error: status === 'error' ? new Error('stale') : null,
          },
          port,
        })
      );
      expect(message.includes('model catalog is not ready')).toBe(true);
    }
    expect(calls).toBe(0);
  });
});
