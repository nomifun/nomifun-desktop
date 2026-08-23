/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { describe, expect, test } from 'bun:test';
import { createTemplateFixture } from '../domain/testFixtures';
import type { CreativeTemplateApi } from './templateApi';
import {
  CreativeTemplateRepositoryError,
  createCreativeTemplateRepository,
} from './templateRepository';

const template = createTemplateFixture();

const apiStub = (overrides: Partial<CreativeTemplateApi> = {}): CreativeTemplateApi => ({
  listTemplates: async () => [template],
  createTemplate: async (definition) => definition,
  getTemplate: async () => template,
  saveTemplate: async (_templateId, request) => request.template,
  deleteTemplate: async () => undefined,
  ...overrides,
});

describe('Creative Template repository', () => {
  test('exposes a revision-safe product persistence port', async () => {
    const calls: unknown[] = [];
    const repository = createCreativeTemplateRepository(
      apiStub({
        saveTemplate: async (templateId, request) => {
          calls.push({ templateId, request });
          return request.template;
        },
      })
    );
    const replacement = {
      ...template,
      revision: 2,
      metadata: { ...template.metadata, name: 'Updated' },
    };

    expect(await repository.list()).toEqual([template]);
    expect(await repository.load(template.id)).toEqual(template);
    expect(await repository.save(template.id, 1, replacement)).toEqual(replacement);
    expect(calls).toEqual([
      {
        templateId: template.id,
        request: { expectedRevision: '1', template: replacement },
      },
    ]);
  });

  test('rejects revision drift before crossing the API boundary', async () => {
    let calls = 0;
    const repository = createCreativeTemplateRepository(
      apiStub({
        saveTemplate: async () => {
          calls += 1;
          return template;
        },
      })
    );
    try {
      await repository.save(template.id, 1, template);
      throw new Error('expected revision rejection');
    } catch (error) {
      expect(error).toMatchObject({ kind: 'invalid-request' });
    }
    expect(calls).toBe(0);
  });

  test('maps stale and missing backend states to stable error kinds', async () => {
    const stale = createCreativeTemplateRepository(
      apiStub({
        saveTemplate: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'PUT',
              path: `/api/creative-studio/templates/${template.id}`,
              status: 409,
              body: { code: 'CONFLICT', error: 'template revision conflict' },
            })
          ),
      })
    );
    try {
      await stale.save(template.id, 1, { ...template, revision: 2 });
      throw new Error('expected conflict');
    } catch (error) {
      expect(error instanceof CreativeTemplateRepositoryError).toBe(true);
      expect(error).toMatchObject({
        kind: 'revision-conflict',
        status: 409,
        backendCode: 'CONFLICT',
      });
    }

    const missing = createCreativeTemplateRepository(
      apiStub({
        getTemplate: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'GET',
              path: `/api/creative-studio/templates/${template.id}`,
              status: 404,
              body: { code: 'NOT_FOUND', error: 'template not found' },
            })
          ),
      })
    );
    try {
      await missing.load(template.id);
      throw new Error('expected missing template');
    } catch (error) {
      expect(error).toMatchObject({ kind: 'not-found', status: 404 });
    }
  });
});
