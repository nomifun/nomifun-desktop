/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeTemplateDefinitionV1, CreativeTemplateRunAggregateV1 } from './types';

export const IDS = {
  template: '018f0000-0000-7000-8000-000000000001',
  variable: '018f0000-0000-7000-8000-000000000002',
  imageVariable: '018f0000-0000-7000-8000-000000000003',
  promptTemplate: '018f0000-0000-7000-8000-000000000004',
  renderStep: '018f0000-0000-7000-8000-000000000005',
  draftStep: '018f0000-0000-7000-8000-000000000006',
  generateStep: '018f0000-0000-7000-8000-000000000007',
  historyStep: '018f0000-0000-7000-8000-000000000008',
  request: '018f0000-0000-7000-8000-000000000009',
  idempotency: '018f0000-0000-7000-8000-00000000000a',
  draft1: '018f0000-0000-7000-8000-00000000000b',
  draft2: '018f0000-0000-7000-8000-00000000000c',
  task: '018f0000-0000-7000-8000-00000000000d',
  asset: '018f0000-0000-7000-8000-00000000000e',
  history: '018f0000-0000-7000-8000-00000000000f',
  provider: '018f0000-0000-7000-8000-000000000010',
  task2: '018f0000-0000-7000-8000-000000000011',
  task3: '018f0000-0000-7000-8000-000000000012',
  result2: '018f0000-0000-7000-8000-000000000013',
} as const;

export function createTemplateFixture(series = false): CreativeTemplateDefinitionV1 {
  const common = {
    id: IDS.template,
    revision: 1,
    metadata: {
      name: 'Product poster',
      description: 'A typed poster template',
      category: 'Marketing',
      visibility: 'private' as const,
      tags: ['poster'],
      createdAt: 1_000,
      updatedAt: 1_000,
    },
    variables: [
      {
        id: IDS.variable,
        key: 'product_name',
        label: 'Product name',
        description: '',
        required: true,
        type: 'text' as const,
        defaultValue: null,
        placeholder: 'Nomi',
        minLength: 1,
        maxLength: 80,
      },
      {
        id: IDS.imageVariable,
        key: 'reference_image',
        label: 'Reference image',
        description: '',
        required: false,
        type: 'image' as const,
        defaultAssetId: null,
      },
    ],
    templates: [
      {
        id: IDS.promptTemplate,
        name: 'Poster prompt',
        segments: [
          { kind: 'text' as const, text: 'Create a poster for ' },
          { kind: 'variable' as const, variableId: IDS.variable },
        ],
      },
    ],
  };
  if (!series) {
    return {
      ...common,
      output: { kind: 'single-image' },
      steps: [
        {
          id: IDS.generateStep,
          kind: 'generate-images',
          name: 'Generate',
          dependsOn: [],
          enabled: true,
          promptSource: { kind: 'template', templateId: IDS.promptTemplate },
          referenceVariableIds: [IDS.imageVariable],
          generation: {
            model: null,
            quality: 'auto',
            width: 1024,
            height: 1024,
            imagesPerPrompt: 1,
          },
        },
        {
          id: IDS.historyStep,
          kind: 'record-history',
          name: 'Record',
          dependsOn: [IDS.generateStep],
          enabled: true,
          sourceStepIds: [IDS.generateStep],
        },
      ],
    };
  }
  return {
    ...common,
    output: { kind: 'multi-image-series', targetCount: 2, concurrency: 2, reviewRequired: true },
    steps: [
      {
        id: IDS.draftStep,
        kind: 'draft-prompts',
        name: 'Draft',
        dependsOn: [],
        enabled: true,
        templateId: IDS.promptTemplate,
        planning: {
          model: null,
          instruction: 'Keep the series coherent.',
          maxTokens: 4096,
        },
      },
      {
        id: IDS.generateStep,
        kind: 'generate-images',
        name: 'Generate',
        dependsOn: [IDS.draftStep],
        enabled: true,
        promptSource: { kind: 'prompt-drafts', stepId: IDS.draftStep },
        referenceVariableIds: [IDS.imageVariable],
        generation: {
          model: null,
          quality: 'auto',
          width: 1024,
          height: 1024,
          imagesPerPrompt: 1,
        },
      },
      {
        id: IDS.historyStep,
        kind: 'record-history',
        name: 'Record',
        dependsOn: [IDS.generateStep],
        enabled: true,
        sourceStepIds: [IDS.generateStep],
      },
    ],
  };
}

export function createExecutableTemplateFixture(series = false): CreativeTemplateDefinitionV1 {
  const template = createTemplateFixture(series);
  for (const step of template.steps) {
    if (step.kind === 'draft-prompts') {
      step.planning.model = {
        providerId: IDS.provider,
        model: 'nomifun-chat-test',
        task: 'chat',
      };
    } else if (step.kind === 'generate-images') {
      step.generation.model = {
        providerId: IDS.provider,
        model: 'nomifun-image-test',
        task: 'image_edit',
      };
    }
  }
  return template;
}

export function createTemplateRunFixture(series = false): CreativeTemplateRunAggregateV1 {
  const template = createExecutableTemplateFixture(series);
  return {
    kind: 'nomifun.creative-studio.template-run',
    version: 1,
    revision: 1,
    templateSnapshot: template,
    request: {
      id: IDS.request,
      idempotencyKey: IDS.request,
      templateId: template.id,
      templateRevision: template.revision,
      requestedAt: 2_000,
      output: template.output.kind === 'single-image'
        ? { kind: 'single-image' }
        : { ...template.output },
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [],
    },
    promptDrafts: [],
    record: {
      requestId: IDS.request,
      templateId: template.id,
      status: 'requested',
      promptDraftIds: [],
      taskIds: [],
      resultAssetIds: [],
      historyReferenceIds: [],
      queuedAt: null,
      startedAt: null,
      completedAt: null,
      failure: null,
    },
  };
}
