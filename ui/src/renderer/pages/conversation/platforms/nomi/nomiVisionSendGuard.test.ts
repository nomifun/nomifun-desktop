/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import type {
  IProvider,
  ModelTask,
  ModelTechnicalCapability,
  ModelTrait,
} from '@/common/config/storage';
import { evaluateNomiVisionSend } from './nomiVisionSendGuard';

const provider = ({
  id,
  model,
  chatTraits = [],
  chatProtocol = 'openai.chat_text',
  unsupportedTechnical = [],
  otherTask,
}: {
  id: string;
  model: string;
  chatTraits?: ModelTrait[];
  chatProtocol?: string;
  unsupportedTechnical?: ModelTechnicalCapability[];
  otherTask?: ModelTask;
}): IProvider =>
  ({
    id,
    platform: 'openai',
    name: id,
    base_url: 'https://example.test/v1',
    auth_scheme: 'bearer',
    has_credentials: false,
    enabled: true,
    models: [
      {
        provider_id: id,
        model,
        enabled: true,
        sort_order: 0,
        created_at: 1,
        updated_at: 1,
        capabilities: [
          {
            task: 'chat',
            traits: chatTraits,
            protocol: chatProtocol,
            connection_role: 'default',
            allow_cross_origin_credentials: false,
            provider_params: {},
            ...(unsupportedTechnical.length > 0
              ? {
                  health: {
                    status: 'unknown' as const,
                    unsupported_technical_capabilities: unsupportedTechnical,
                  },
                }
              : {}),
            created_at: 1,
            updated_at: 1,
          },
          ...(otherTask
            ? [
                {
                  task: otherTask,
                  traits: ['vision_input'] as ModelTrait[],
                  protocol: 'test.other',
                  connection_role: 'default',
                  allow_cross_origin_credentials: false,
                  provider_params: {},
                  created_at: 1,
                  updated_at: 1,
                },
              ]
            : []),
        ],
      },
    ],
  }) as IProvider;

const decision = ({
  providers,
  providerId = 'provider-a',
  model = 'same-model',
  files = ['C:/tmp/photo.PNG'],
  providerGraphResolved = true,
  visionModel,
}: {
  providers: IProvider[];
  providerId?: string;
  model?: string;
  files?: string[];
  providerGraphResolved?: boolean;
  visionModel?: { provider_id: string; model: string };
}) =>
  evaluateNomiVisionSend({ providers, providerId, model, files, providerGraphResolved, visionModel });

describe('Nomi image-send capability guard', () => {
  test('allows images on an exact Chat adapter without requiring manual vision metadata', () => {
    expect(
      decision({
        providers: [
          provider({ id: 'provider-a', model: 'same-model', chatTraits: ['vision_input'] }),
        ],
      })
    ).toEqual({ allowed: true });

    expect(decision({ providers: [provider({ id: 'provider-a', model: 'same-model' })] }))
      .toEqual({ allowed: true });
  });

  test('does not use names, unrelated routes, or stale traits to invent missing image encoding', () => {
    const selected = provider({
      id: 'provider-a', model: 'gpt-4o', chatProtocol: 'unknown.chat',
      chatTraits: ['vision_input'], otherTask: 'image_generation',
    });
    const otherProvider = provider({
      id: 'provider-b',
      model: 'gpt-4o',
      chatTraits: ['vision_input'],
    });
    const otherModel = provider({
      id: 'provider-a',
      model: 'other-model',
      chatTraits: ['vision_input'],
    });

    expect(
      decision({ providers: [selected, otherProvider, otherModel], model: 'gpt-4o' })
    ).toEqual({ allowed: false, reason: 'vision_not_supported' });
  });

  test('allows an exact untagged vision fallback while preserving observed tool-call exclusions', () => {
    expect(
      decision({
        providers: [
          provider({ id: 'provider-a', model: 'text-only', chatProtocol: 'unknown.chat' }),
          provider({ id: 'provider-b', model: 'vision' }),
        ],
        model: 'text-only',
        visionModel: { provider_id: 'provider-b', model: 'vision' },
      })
    ).toEqual({ allowed: true });

    expect(
      decision({
        providers: [
          provider({ id: 'provider-a', model: 'text-only', chatProtocol: 'unknown.chat' }),
          provider({
            id: 'provider-b',
            model: 'vision-only',
            chatTraits: ['vision_input'],
            unsupportedTechnical: ['function_calling'],
          }),
        ],
        model: 'text-only',
        visionModel: { provider_id: 'provider-b', model: 'vision-only' },
      })
    ).toEqual({ allowed: false, reason: 'vision_not_supported' });
  });

  test('requires the selected model to have a configured Chat route', () => {
    expect(decision({
      providers: [provider({ id: 'provider-a', model: 'different-model', chatTraits: ['vision_input'] })],
    })).toEqual({ allowed: false, reason: 'vision_not_supported' });
    const imageOnly = provider({ id: 'provider-a', model: 'same-model', otherTask: 'image_generation' });
    imageOnly.models[0]!.capabilities = imageOnly.models[0]!.capabilities.filter(capability => capability.task !== 'chat');
    expect(decision({ providers: [imageOnly] })).toEqual({ allowed: false, reason: 'vision_not_supported' });
  });

  test('fails closed while the provider capability graph is unresolved', () => {
    expect(decision({ providers: [], providerGraphResolved: false })).toEqual({
      allowed: false,
      reason: 'capability_unavailable',
    });
  });

  test('does not constrain ordinary messages without image attachments', () => {
    expect(
      decision({ providers: [], providerGraphResolved: false, files: ['C:/tmp/notes.pdf'] })
    ).toEqual({ allowed: true });
  });
});

describe('NomiSendBox blocking wiring', () => {
  const source = readFileSync(new URL('./NomiSendBox.tsx', import.meta.url), 'utf8');

  test('gates normal, edit-resubmit, queued/initial, and steer sends before mutation or IPC', () => {
    const normal = source.slice(
      source.indexOf('const onSendHandler'),
      source.indexOf('const handleEditResubmit')
    );
    expect(normal.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeGreaterThan(-1);
    expect(normal.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeLessThan(
      normal.indexOf('clearFiles()')
    );

    const execute = source.slice(
      source.indexOf('const executeCommand'),
      source.indexOf('const onSendHandler')
    );
    expect(execute.indexOf('if (!canSendFiles(files))')).toBeGreaterThan(-1);
    expect(execute.indexOf('if (!canSendFiles(files))')).toBeLessThan(
      execute.indexOf('ipcBridge.conversation.sendMessage.invoke')
    );

    const edit = source.slice(
      source.indexOf('const handleEditResubmit'),
      source.indexOf('const executeSteer')
    );
    expect(edit.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeGreaterThan(-1);
    expect(edit.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeLessThan(
      edit.indexOf('ipcBridge.conversation.sendMessage.invoke')
    );

    const steer = source.slice(
      source.indexOf('const onSteerHandler'),
      source.indexOf('const handleEditQueuedCommand')
    );
    expect(steer.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeGreaterThan(-1);
    expect(steer.indexOf('if (!canSendFiles(filesToSend)) return;')).toBeLessThan(
      steer.indexOf('executeSteer')
    );
  });

  test('reads the provider graph directly and has no platform/name inference fallback', () => {
    expect(source.includes('useProvidersQuery()')).toBe(true);
    expect(source.includes('evaluateNomiVisionSend({')).toBe(true);
    expect(source.includes('useModelsForTask')).toBe(false);
    expect(source.includes('maybeWarnNonVisionModel')).toBe(false);
  });
});
