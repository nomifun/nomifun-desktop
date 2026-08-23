/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { parseProviderId } from '@/common/types/ids';

import { CreativeStudioContractError } from '../../domain';
import { validateTemplateDefinition } from '../domain';

import {
  CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND,
  type CreativeTemplateDraftArtifact,
} from './artifacts';
import { convertCreativeTemplateDraft } from './converter';

const PROVIDER_ID = parseProviderId('0190f5fe-7c00-7a00-8000-000000000901');
const chatModel = { providerId: PROVIDER_ID, model: 'nomi-chat' } as const;
const UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

const artifact = (
  mode: 'single-image' | 'multi-image-series',
  promptTemplate: string
): CreativeTemplateDraftArtifact => ({
  kind: CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND,
  summary: '草稿已生成',
  draft: {
    mode,
    name: mode === 'single-image' ? '商品主图' : '社媒多图',
    description: '最小可上线模板',
    category: '营销',
    promptTemplate,
  },
});

const renderTemplateKeys = (
  template: ReturnType<typeof convertCreativeTemplateDraft>
): string => {
  const keys = new Map(template.variables.map((variable) => [variable.id, variable.key]));
  return template.templates[0].segments
    .map((segment) =>
      segment.kind === 'text'
        ? segment.text
        : `{{${keys.get(segment.variableId) ?? 'missing'}}}`
    )
    .join('');
};

const expectConversionFailure = (
  value: CreativeTemplateDraftArtifact
): CreativeStudioContractError => {
  try {
    convertCreativeTemplateDraft(value, chatModel);
    throw new Error('Expected template draft conversion failure');
  } catch (error) {
    expect(error instanceof CreativeStudioContractError).toBe(true);
    return error as CreativeStudioContractError;
  }
};

describe('Creative Template Agent draft converter', () => {
  test('builds a valid private single-image definition from the fixed blank template', () => {
    const prompt = '为 {{product_name}} 生成主图，突出 {{selling_points}}。';
    const template = convertCreativeTemplateDraft(artifact('single-image', prompt), chatModel);

    expect(validateTemplateDefinition(template).ok).toBe(true);
    expect(UUID_V7.test(template.id)).toBe(true);
    expect(template.revision).toBe(1);
    expect(template.metadata).toEqual({
      name: '商品主图',
      description: '最小可上线模板',
      category: '营销',
      visibility: 'private',
      tags: [],
      createdAt: 0,
      updatedAt: 0,
    });
    expect(template.variables.map((variable) => variable.key)).toEqual([
      'product_name',
      'selling_points',
    ]);
    expect(renderTemplateKeys(template)).toBe(prompt);
    expect(
      template.steps.find((step) => step.kind === 'generate-images')?.generation.model
    ).toBeNull();
    expect(template.steps.some((step) => step.kind === 'draft-prompts')).toBe(false);
    expect(JSON.stringify(template).includes(chatModel.model)).toBe(false);
  });

  test('injects only the exact turn Chat model into multi-image prompt planning', () => {
    const prompt =
      '围绕 {{ topic }} 规划一组图片，风格为 {{style}}，适配 {{platform}}。';
    const template = convertCreativeTemplateDraft(
      artifact('multi-image-series', prompt),
      chatModel
    );

    expect(validateTemplateDefinition(template).ok).toBe(true);
    expect(template.variables.map((variable) => variable.key)).toEqual([
      'topic',
      'style',
      'platform',
    ]);
    expect(renderTemplateKeys(template)).toBe(
      '围绕 {{topic}} 规划一组图片，风格为 {{style}}，适配 {{platform}}。'
    );
    const planner = template.steps.find((step) => step.kind === 'draft-prompts');
    expect(planner?.planning.model).toEqual({
      providerId: PROVIDER_ID,
      model: 'nomi-chat',
      task: 'chat',
    });
    expect(
      template.steps.find((step) => step.kind === 'generate-images')?.generation.model
    ).toBeNull();
  });

  test('rejects missing, unknown, unclosed, nested and unmatched placeholders', () => {
    expectConversionFailure(
      artifact('single-image', '生成一张没有变量的固定主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{unknown}} 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{product_name 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 {{product_{{name}} 生成主图')
    );
    expectConversionFailure(
      artifact('single-image', '为 product_name}} 生成主图')
    );
  });

  test('runs final template validation before returning the definition', () => {
    const error = expectConversionFailure(
      artifact('single-image', '{{product_name}}'.repeat(501))
    );
    expect(error.path).toBe('$.templates[0].segments');
  });
});
