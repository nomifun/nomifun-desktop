/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseMiniAppId } from '@/common/types/ids';
import {
  MINI_APP_BUILDER_SYSTEM_PROMPT,
  MINI_APP_EXTRA_FLAG,
  MINI_APP_FILE_NAME,
  MINI_APP_IFRAME_SANDBOX,
  MINI_APP_NAME_SNIPPET_LENGTH,
  isMiniAppConversation,
  resolveMiniAppServeUrl,
} from './contract';

const MINI_APP_ID = parseMiniAppId('0198f3b2-4c1a-7c3d-8e9f-0a1b2c3d4e5f');

describe('mini-app contract', () => {
  test('pins the single-artifact file name and the one extra marker key', () => {
    expect(MINI_APP_FILE_NAME).toBe('miniapp.html');
    expect(MINI_APP_EXTRA_FLAG).toBe('miniapp');
    expect(MINI_APP_NAME_SNIPPET_LENGTH).toBe(16);
  });

  test('isMiniAppConversation only accepts an explicit boolean-true marker', () => {
    expect(isMiniAppConversation({ [MINI_APP_EXTRA_FLAG]: true })).toBe(true);
    // Tolerates unknown neighbours in the loose `extra` bag.
    expect(isMiniAppConversation({ miniapp: true, system_prompt: 'x' })).toBe(true);

    expect(isMiniAppConversation({ miniapp: 'true' })).toBe(false);
    expect(isMiniAppConversation({ miniapp: 1 })).toBe(false);
    expect(isMiniAppConversation({ miniapp: false })).toBe(false);
    expect(isMiniAppConversation({ system_prompt: 'x' })).toBe(false);
    expect(isMiniAppConversation({})).toBe(false);
    expect(isMiniAppConversation(null)).toBe(false);
    expect(isMiniAppConversation(undefined)).toBe(false);
    expect(isMiniAppConversation('miniapp')).toBe(false);
    expect(isMiniAppConversation(0)).toBe(false);
  });

  test('serve URL targets the unauthenticated per-app route', () => {
    const url = resolveMiniAppServeUrl(MINI_APP_ID);
    expect(url.endsWith(`/api/miniapps/${MINI_APP_ID}/serve`)).toBe(true);
    // Runner and preview iframes need an absolute origin in the desktop shell.
    expect(url.startsWith('http://127.0.0.1:')).toBe(true);
  });

  test('builder prompt pins the artifact path and demands self-containment', () => {
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes(MINI_APP_FILE_NAME)).toBe(true);
    // Self-containment: inline CSS/JS, no sibling workspace files.
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('自包含')).toBe(true);
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('内联全部 CSS 与 JavaScript')).toBe(true);
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('不得依赖工作区内其他文件')).toBe(true);
  });

  test('builder prompt requires storage to degrade gracefully in the sandbox', () => {
    // The sandbox has an opaque origin, so `localStorage` access may throw.
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('localStorage')).toBe(true);
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('try/catch')).toBe(true);
    expect(MINI_APP_BUILDER_SYSTEM_PROMPT.includes('核心功能不得依赖持久化')).toBe(true);
  });

  test('iframe sandbox withholds same-origin so generated code gets an opaque origin', () => {
    expect(MINI_APP_IFRAME_SANDBOX).toBe('allow-scripts allow-forms allow-popups allow-modals');
    // `allow-scripts` + `allow-same-origin` together void the sandbox entirely.
    expect(MINI_APP_IFRAME_SANDBOX.includes('allow-same-origin')).toBe(false);
  });
});
