/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import { evaluateNomiVisionSend } from './nomiVisionSendGuard';

const provider = ({
  id,
  model,
  chatTraits = [],
  otherTask,
}: {
  id: string;
  model: string;
  chatTraits?: ModelTrait[];
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
            protocol: 'openai.chat_text',
            connection_role: 'default',
            allow_cross_origin_credentials: false,
            provider_params: {},
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
}: {
  providers: IProvider[];
  providerId?: string;
  model?: string;
  files?: string[];
  providerGraphResolved?: boolean;
}) =>
  evaluateNomiVisionSend({ providers, providerId, model, files, providerGraphResolved });

describe('Nomi image-send capability guard', () => {
  test('allows images only when the exact provider/model Chat capability declares vision_input', () => {
    expect(
      decision({
        providers: [
          provider({ id: 'provider-a', model: 'same-model', chatTraits: ['vision_input'] }),
        ],
      })
    ).toEqual({ allowed: true });

    expect(decision({ providers: [provider({ id: 'provider-a', model: 'same-model' })] })).toEqual({
      allowed: false,
      reason: 'vision_not_supported',
    });
  });

  test('never infers vision from platform, model name, another provider, model, or task', () => {
    const selected = provider({ id: 'provider-a', model: 'gpt-4o', otherTask: 'image_generation' });
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
      edit.indexOf('ipcBridge.conversation.editResubmit.invoke')
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
