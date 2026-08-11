/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelResponse } from '@/common/protocolBindings/ProviderModelResponse';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { NOMIFUN_FREE_MODEL_PLATFORM } from '@/common/types/provider/managedModelService';
import { orderModelSelectorProviders } from '@/renderer/hooks/agent/modelSelectorProviderOrdering';
import {
  buildModalityGroups,
  buildUntaggedGroups,
  isUntaggedRow,
  MODALITY_SPECS,
  rowMatchesModality,
} from './modalityModels';

const A = '0190f5fe-7c00-7a00-8000-0000000000a1' as ProviderId;
const B = '0190f5fe-7c00-7a00-8000-0000000000b2' as ProviderId;
const FREE = '0190f5fe-7c00-7a00-8000-0000000000f3' as ProviderId;

const provider = (id: ProviderId, name: string): IProvider =>
  ({ id, name, platform: 'custom', enabled: true }) as unknown as IProvider;

const freeProvider = (id: ProviderId): IProvider =>
  ({ id, name: 'NomiFun Free', platform: NOMIFUN_FREE_MODEL_PLATFORM, enabled: true }) as unknown as IProvider;

const row = (
  providerId: ProviderId,
  model: string,
  overrides: Partial<ProviderModelResponse> = {}
): ProviderModelResponse =>
  ({
    provider_id: providerId,
    model,
    enabled: true,
    sort_order: 0,
    tasks: ['chat'],
    traits: [],
    params: {},
    source: 'inferred',
    created_at: 0,
    updated_at: 0,
    ...overrides,
  }) as ProviderModelResponse;

describe('modality specs', () => {
  test('vision is a trait-filtered chat projection, not its own task', () => {
    expect(MODALITY_SPECS.vision.tasks).toEqual(['chat']);
    expect(MODALITY_SPECS.vision.traits).toEqual(['vision_input']);
    expect(MODALITY_SPECS.chat.traits).toEqual([]);
    expect(MODALITY_SPECS.embedding.tasks).toEqual(['embedding', 'rerank']);
  });

  test('a row matches when it owns ANY listed task and EVERY listed trait', () => {
    expect(rowMatchesModality(row(A, 'm'), MODALITY_SPECS.chat)).toBe(true);
    expect(rowMatchesModality(row(A, 'm'), MODALITY_SPECS.vision)).toBe(false);
    expect(
      rowMatchesModality(row(A, 'm', { traits: ['vision_input'] }), MODALITY_SPECS.vision)
    ).toBe(true);
    expect(rowMatchesModality(row(A, 'e', { tasks: ['rerank'] }), MODALITY_SPECS.embedding)).toBe(
      true
    );
    expect(rowMatchesModality(row(A, 'e', { tasks: ['rerank'] }), MODALITY_SPECS.chat)).toBe(false);
  });

  test('disabled rows stay visible so the section can switch them back on', () => {
    // The projection reads `provider_models` rows directly rather than the
    // resolve endpoint precisely for this: resolve only ever returns ENABLED
    // rows, so a toggle built on it could only ever turn things off and then
    // lose sight of them.
    // `sort_order` is spelled out here: this case is about a disabled row still
    // being listed, not about how a sort_order tie is broken (that is the next
    // case).
    const groups = buildModalityGroups(
      [row(A, 'on', { sort_order: 1 }), row(A, 'off', { sort_order: 2, enabled: false })],
      [provider(A, 'A')],
      MODALITY_SPECS.chat
    );
    expect(groups[0].models.map((m) => m.model)).toEqual(['on', 'off']);
    expect(groups[0].models[1].enabled).toBe(false);
  });

  test('groups follow provider order, models follow sort_order then name', () => {
    const rows = [
      row(B, 'b2', { sort_order: 1 }),
      row(A, 'a2', { sort_order: 2 }),
      row(A, 'a1', { sort_order: 1 }),
      row(B, 'b1', { sort_order: 1 }),
    ];
    const groups = buildModalityGroups(
      rows,
      [provider(A, 'A'), provider(B, 'B')],
      MODALITY_SPECS.chat
    );
    expect(groups.map((g) => g.providerId)).toEqual([A, B]);
    expect(groups[0].models.map((m) => m.model)).toEqual(['a1', 'a2']);
    expect(groups[1].models.map((m) => m.model)).toEqual(['b1', 'b2']);
  });

  test('rows of an unknown provider are dropped, and an empty provider yields no group', () => {
    const groups = buildModalityGroups([row(B, 'orphan')], [provider(A, 'A')], MODALITY_SPECS.chat);
    expect(groups).toEqual([]);
  });

  test('untagged rows are collected instead of silently vanishing', () => {
    // A legacy row with `tasks: []` matches no modality. Hiding it would make a
    // configured model invisible on every page of the hub; the 对话 section shows
    // it in an explicit "untagged" bucket with a pointer to the tag editor.
    expect(isUntaggedRow(row(A, 'x', { tasks: [] }))).toBe(true);
    expect(isUntaggedRow(row(A, 'x'))).toBe(false);
    const groups = buildUntaggedGroups(
      [row(A, 'tagged'), row(A, 'bare', { tasks: [] })],
      [provider(A, 'A')]
    );
    expect(groups[0].models.map((m) => m.model)).toEqual(['bare']);
  });

  test('a custom provider label is applied (free-model platform renaming)', () => {
    const groups = buildModalityGroups(
      [row(A, 'm')],
      [provider(A, 'raw')],
      MODALITY_SPECS.chat,
      () => '免费模型'
    );
    expect(groups[0].providerName).toBe('免费模型');
  });

  test('the free-model group sits BELOW the groups the user configured', () => {
    // The backend lists the managed free provider FIRST — it is auto-created
    // before the user has added anything. A management view must not lead with
    // models the user never configured, so the panel feeds these groups the
    // shared selector ordering (which ranks the free platform last) instead of
    // the raw provider query.
    const groups = buildModalityGroups(
      [row(FREE, 'free-m'), row(A, 'a'), row(B, 'b')],
      orderModelSelectorProviders([freeProvider(FREE), provider(A, 'A'), provider(B, 'B')]),
      MODALITY_SPECS.chat
    );
    expect(groups.map((g) => g.providerId)).toEqual([A, B, FREE]);
  });

  test('untagged rows follow the same ordering, free last', () => {
    const groups = buildUntaggedGroups(
      [row(FREE, 'free-bare', { tasks: [] }), row(A, 'bare', { tasks: [] })],
      orderModelSelectorProviders([freeProvider(FREE), provider(A, 'A')])
    );
    expect(groups.map((g) => g.providerId)).toEqual([A, FREE]);
  });
});
