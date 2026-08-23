/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createTemplateFixture } from '../domain/testFixtures';
import {
  CreativeTemplateContractError,
  createCreativeTemplateApi,
  type CreativeTemplateHttpRequest,
} from './templateApi';

describe('Creative Template API client', () => {
  test('uses the canonical CRUD paths and exact wire bodies', async () => {
    const original = createTemplateFixture();
    const replacement = {
      ...createTemplateFixture(),
      revision: 2,
      metadata: { ...original.metadata, name: 'Updated template' },
    };
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeTemplateHttpRequest = async (method, path, body) => {
      calls.push({ method, path, ...(body === undefined ? {} : { body }) });
      if (method === 'DELETE') return undefined;
      if (method === 'GET' && path.endsWith('/templates')) {
        return { templates: [original] };
      }
      return { template: method === 'PUT' ? replacement : original };
    };
    const api = createCreativeTemplateApi(request);

    expect(await api.listTemplates()).toEqual([original]);
    expect(await api.createTemplate(original)).toEqual(original);
    expect(await api.getTemplate(original.id)).toEqual(original);
    expect(
      await api.saveTemplate(original.id, {
        expectedRevision: '1',
        template: replacement,
      })
    ).toEqual(replacement);
    await api.deleteTemplate(original.id);

    expect(calls).toEqual([
      { method: 'GET', path: '/api/creative-studio/templates' },
      {
        method: 'POST',
        path: '/api/creative-studio/templates',
        body: { template: original },
      },
      { method: 'GET', path: `/api/creative-studio/templates/${original.id}` },
      {
        method: 'PUT',
        path: `/api/creative-studio/templates/${original.id}`,
        body: { expectedRevision: '1', template: replacement },
      },
      { method: 'DELETE', path: `/api/creative-studio/templates/${original.id}` },
    ]);
  });

  test('fails closed on unknown response fields and route identity drift', async () => {
    const template = createTemplateFixture();
    const unknown = createCreativeTemplateApi(async () => ({
      templates: [{ ...template, legacyPrompt: 'unsafe' }],
    }));
    try {
      await unknown.listTemplates();
      throw new Error('expected unknown-field rejection');
    } catch (error) {
      expect(error instanceof CreativeTemplateContractError).toBe(true);
      expect(error).toMatchObject({ code: 'unknown-field' });
    }

    const drifted = createCreativeTemplateApi(async () => ({
      template: {
        ...template,
        id: '018f0000-0000-7000-8000-000000000099',
      },
    }));
    try {
      await drifted.getTemplate(template.id);
      throw new Error('expected route identity rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'identity-mismatch', path: '$.template.id' });
    }
  });

  test('validates writes before issuing a request', async () => {
    const template = createTemplateFixture();
    let calls = 0;
    const api = createCreativeTemplateApi(async () => {
      calls += 1;
      return { template };
    });
    try {
      await api.saveTemplate(template.id, {
        expectedRevision: '01',
        template: { ...template, revision: 2 },
      });
      throw new Error('expected invalid revision rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'invalid-value', path: '$.expectedRevision' });
    }
    expect(calls).toBe(0);
  });
});
