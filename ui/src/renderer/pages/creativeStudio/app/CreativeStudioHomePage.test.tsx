/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderId } from '@/common/types/ids';
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreateCreativeProjectRequest, CreativeProjectSummary } from '../domain';
import type { CreativeProjectRepository } from '../services/projectRepository';
import {
  CreativeStudioHomeSurface,
  createCreativeStudioHomeProject,
  creativeStudioHomeProjectTitle,
} from './CreativeStudioHomePage';

const PROJECT_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const PROVIDER_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f118' as ProviderId;
const project: CreativeProjectSummary = {
  projectId: PROJECT_ID,
  title: '新品海报',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 1,
};

const repositoryFixture = (create: CreativeProjectRepository['create']) =>
  ({
    list: async () => [],
    create,
    load: async () => {
      throw new Error('unused');
    },
    save: async () => {
      throw new Error('unused');
    },
    rename: async () => {
      throw new Error('unused');
    },
    remove: async () => undefined,
  }) satisfies CreativeProjectRepository;

describe('Creative Studio minimal home', () => {
  test('renders only the launch fields, loading state, and inline error', () => {
    const html = renderToStaticMarkup(
      <CreativeStudioHomeSurface
        prompt='新品海报'
        canSubmit={false}
        submitting
        error='所选模型已不可用'
        modelSelector={<div data-test-model-selector>model</div>}
        onPromptChange={() => undefined}
        onSubmit={() => undefined}
      />
    );

    expect(html.includes('data-creative-studio-home="true"')).toBe(true);
    expect(html.includes('<textarea')).toBe(true);
    expect(html.includes('maxLength="65536"')).toBe(true);
    expect(html.includes('data-test-model-selector="true"')).toBe(true);
    expect(html.includes('aria-busy="true"')).toBe(true);
    expect(html.includes('正在创建…')).toBe(true);
    expect(html.includes('role="alert"')).toBe(true);
    expect(html.includes('所选模型已不可用')).toBe(true);
  });

  test('derives a short Unicode title and navigates only after exact kickoff creation', async () => {
    const requests: CreateCreativeProjectRequest[] = [];
    const paths: string[] = [];
    const prompt = `  ${'😀'.repeat(25)}补充文字  `;
    const repository = repositoryFixture(async (request = {}) => {
      requests.push(request);
      expect(paths).toEqual([]);
      return project;
    });

    expect(creativeStudioHomeProjectTitle(prompt)).toBe('😀'.repeat(24));
    await createCreativeStudioHomeProject({
      prompt,
      model: { providerId: PROVIDER_ID, model: 'gpt-5' },
      repository,
      navigate: (path) => paths.push(path),
    });

    expect(requests).toEqual([
      {
        title: '😀'.repeat(24),
        agentKickoff: {
          prompt: `${'😀'.repeat(25)}补充文字`,
          model: { providerId: PROVIDER_ID, model: 'gpt-5' },
        },
      },
    ]);
    expect(paths).toEqual([`/workshop/canvas/${PROJECT_ID}`]);
  });

  test('does not create for blank input or a missing model and never navigates on failure', async () => {
    let creates = 0;
    const paths: string[] = [];
    const idleRepository = repositoryFixture(async () => {
      creates += 1;
      return project;
    });

    expect(
      await createCreativeStudioHomeProject({
        prompt: '   ',
        model: { providerId: PROVIDER_ID, model: 'gpt-5' },
        repository: idleRepository,
        navigate: (path) => paths.push(path),
      })
    ).toBeNull();
    expect(
      await createCreativeStudioHomeProject({
        prompt: '新品海报',
        model: null,
        repository: idleRepository,
        navigate: (path) => paths.push(path),
      })
    ).toBeNull();
    expect(creates).toBe(0);

    const failedRepository = repositoryFixture(async () => {
      throw new Error('offline');
    });
    try {
      await createCreativeStudioHomeProject({
        prompt: '新品海报',
        model: { providerId: PROVIDER_ID, model: 'gpt-5' },
        repository: failedRepository,
        navigate: (path) => paths.push(path),
      });
      throw new Error('Expected create failure');
    } catch (error) {
      expect((error as Error).message).toBe('offline');
    }
    expect(paths).toEqual([]);
  });

  test('keeps the launch design fixed, exact-model scoped, and intentionally small', () => {
    const source = readFileSync(new URL('./CreativeStudioHomePage.tsx', import.meta.url), 'utf8');
    const css = readFileSync(new URL('./CreativeStudioHomePage.module.css', import.meta.url), 'utf8');

    expect(source.includes("task: 'chat'")).toBe(true);
    expect(source.includes('onOpenModelSettings')).toBe(true);
    expect(source.includes("navigate('/models?section=models')")).toBe(true);
    expect(source.includes('agentKickoff')).toBe(true);
    expect(source.includes('attachment')).toBe(false);
    expect(source.includes('sessionHistory')).toBe(false);
    expect(css.includes('--color-bg-1: #f4f2ed')).toBe(true);
    expect(css.includes('color-scheme: light')).toBe(true);
  });
});
