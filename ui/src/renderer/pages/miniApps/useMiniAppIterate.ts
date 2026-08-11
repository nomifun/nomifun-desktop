/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 「继续迭代」 — shared by the library card and the runner toolbar (spec D19).
 *
 * Two steps, in this order and never merged:
 *
 *  1. `POST /api/miniapps/{id}/workspace` materializes the working copy and
 *     answers its ABSOLUTE path. Idempotent, and it creates no conversation.
 *  2. an ORDINARY Nomi conversation is launched through the same
 *     {@link useNomiQuickStart} the creation path uses — ordinary managed
 *     workspace, no marker, no redirection — whose first message is the one
 *     {@link buildMiniAppIterateMessage} composes.
 *
 * The mini-app is NOT bound to that conversation: it is one of the user's own
 * threads, it appears in the session list, and deleting it leaves the app (and
 * its source on disk) untouched. Publishing stays an explicit act on
 * `/mini-apps/:id`, which is what the first message tells the user.
 *
 * Design spec: docs/specs/2026-08-10-miniapps-v3-unified-conversations.zh.md
 */
import { useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import { useNomiQuickStart } from '@renderer/hooks/agent/useNomiQuickStart';
import {
  buildMiniAppIterateConversationName,
  buildMiniAppIterateMessage,
} from './contract';

/** The app being iterated on — the two fields the first message quotes. */
export type MiniAppIterateSubject = Pick<IApiMiniApp, 'miniapp_id' | 'name'>;

export const useMiniAppIterate = () => {
  const { t } = useTranslation();
  const { start } = useNomiQuickStart();
  const [starting, setStarting] = useState(false);
  // Synchronous guard: `starting` is state, so two clicks inside one tick would
  // both pass it and create two conversations for the same app.
  const startingRef = useRef(false);

  const iterate = useCallback(
    async (app: MiniAppIterateSubject): Promise<boolean> => {
      if (startingRef.current) return false;
      startingRef.current = true;
      setStarting(true);
      try {
        const workspace = await ipcBridge.miniapps.provisionWorkspace.invoke({
          miniapp_id: app.miniapp_id,
        });
        const sourcePath = workspace?.source_path?.trim();
        if (!sourcePath) {
          Message.error(t('miniApps.iterate.failed'));
          return false;
        }
        // `start` raises its own message and navigates on success.
        return await start({
          name: buildMiniAppIterateConversationName(app.name, t),
          prompt: buildMiniAppIterateMessage(
            { name: app.name, miniAppId: app.miniapp_id, sourcePath },
            t
          ),
        });
      } catch (error) {
        console.error('[miniapps] failed to start an iteration conversation', error);
        const detail = isBackendHttpError(error) && error.backendMessage.trim() ? error.backendMessage : '';
        Message.error(detail ? t('miniApps.iterate.failedDetail', { detail }) : t('miniApps.iterate.failed'));
        return false;
      } finally {
        startingRef.current = false;
        setStarting(false);
      }
    },
    [start, t]
  );

  return useMemo(() => ({ iterate, starting }), [iterate, starting]);
};
