/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TProviderWithModel } from '@/common/config/storage';
import { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import {
  MINI_APP_BUILDER_SYSTEM_PROMPT,
  MINI_APP_EXTRA_FLAG,
  MINI_APP_NAME_SNIPPET_LENGTH,
} from '@renderer/pages/miniApps/contract';
import { useNomiQuickStart } from './useNomiQuickStart';

export interface MiniAppQuickStartOptions {
  /** The user's request, sent as the first message of the new conversation. */
  prompt: string;
  /**
   * Model the caller's picker resolved. Required in practice on the start page:
   * a second `useGuidModelSelection` instance would not observe the user's pick.
   */
  model?: TProviderWithModel;
  /** Staged workspace directory, if the user chose one. */
  dir?: string;
  /** Staged attachments, mirroring `extra.default_files` on a normal launch. */
  files?: string[];
}

/** Conversation title: the builder marker plus a short quote of the request. */
export const miniAppConversationNameSnippet = (prompt: string): string =>
  prompt.trim().slice(0, MINI_APP_NAME_SNIPPET_LENGTH);

/**
 * Launch a mini-app builder conversation from the start page.
 *
 * A thin wrapper over {@link useNomiQuickStart} — engine is pinned to Nomi per
 * spec D4, so the only difference is the create call's `extra`: the builder
 * instructions ride `extra.system_prompt` (the Nomi engine's `custom` prompt
 * section, which the backend neither strips nor regenerates, so reopening the
 * session keeps the behavior) and the `miniapp` marker lets the conversation
 * surface turn on auto-preview and the solidify toolbar. Everything else
 * (create → history refresh → initial-message handoff → navigate) is shared.
 */
export const useMiniAppQuickStart = () => {
  const { t } = useTranslation();
  const { start: startNomi, canStart } = useNomiQuickStart();

  const start = useCallback(
    ({ prompt, model, dir, files }: MiniAppQuickStartOptions): Promise<boolean> =>
      startNomi({
        name: t('miniApps.composer.conversationName', {
          name: miniAppConversationNameSnippet(prompt),
        }),
        prompt,
        model,
        extra: {
          workspace: dir || '',
          custom_workspace: Boolean(dir),
          default_files: files ?? [],
          system_prompt: MINI_APP_BUILDER_SYSTEM_PROMPT,
          [MINI_APP_EXTRA_FLAG]: true,
        },
      }),
    [startNomi, t]
  );

  return useMemo(() => ({ start, canStart }), [start, canStart]);
};
