/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { joinPath } from '@/common/chat/chatLib';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { MINI_APP_FILE_NAME, isMiniAppConversation } from '@renderer/pages/miniApps/contract';
import { useEffect, useRef } from 'react';
import useSWR from 'swr';

/**
 * Auto-opens the mini-app preview tab for a mini-app conversation.
 *
 * Mini-app sessions have exactly one artifact — `miniapp.html` in the workspace
 * root — so there is nothing to choose: as soon as the file exists the preview
 * belongs on screen. Probes on mount (reopening a session restores the tab even
 * though `miniapp` is not a persisted preview type) and after every completed
 * turn of THIS conversation, which is the authoritative "the agent finished
 * rewriting the file" signal. Once a tab is open the existing
 * `fileStream.contentUpdate` subscription plus the 1s mtime poll keep it fresh,
 * so `findPreviewTab` short-circuits every later trigger.
 */
export const useAutoPreviewMiniApp = (
  conversation: Pick<ConversationContextValue, 'conversation_id' | 'workspace'> | null
) => {
  const { findPreviewTab } = usePreviewContext();
  const { launchPreview } = usePreviewLauncher();
  const conversation_id = conversation?.conversation_id;
  const workspace = conversation?.workspace?.trim() ? conversation.workspace : undefined;
  const probingRef = useRef(false);

  // Both callbacks change identity on every preview-tab mutation (`findPreviewTab`
  // closes over the tab list). Parking them in refs keeps the effect below keyed
  // on conversation identity alone, so the `turnCompleted` subscription is set up
  // once per conversation instead of on every tab open/close/content tick.
  const findPreviewTabRef = useRef(findPreviewTab);
  findPreviewTabRef.current = findPreviewTab;
  const launchPreviewRef = useRef(launchPreview);
  launchPreviewRef.current = launchPreview;

  // Reuses the conversation route's own SWR key, so the `extra` flag is read
  // from cache rather than costing an extra request.
  const { data: record } = useSWR(conversation_id ? `conversation/${conversation_id}` : null, () =>
    getConversationOrNull(conversation_id!)
  );
  const isMiniApp = isMiniAppConversation(record?.extra);

  useEffect(() => {
    if (!isMiniApp || !workspace || !conversation_id) return;

    const file_path = joinPath(workspace, MINI_APP_FILE_NAME);

    const probeAndOpen = async () => {
      if (probingRef.current) return;
      // Dedupe first: an already-open tab tracks the file on its own, so a
      // repeat trigger must cost zero IPC.
      if (findPreviewTabRef.current('miniapp', '', { file_path, file_name: MINI_APP_FILE_NAME })) return;
      probingRef.current = true;
      try {
        const metadata = await ipcBridge.fs.getFileMetadata.invoke({ path: file_path, workspace });
        if (!metadata || metadata.isDirectory) return;
        await launchPreviewRef.current({
          relativePath: MINI_APP_FILE_NAME,
          contentType: 'miniapp',
          editable: false,
        });
      } catch {
        // The agent has not written the file yet (or it was deleted) — stay inert.
      } finally {
        probingRef.current = false;
      }
    };

    void probeAndOpen();

    return ipcBridge.conversation.turnCompleted.on((event) => {
      if (event.conversation_id !== conversation_id) return;
      void probeAndOpen();
    });
  }, [isMiniApp, workspace, conversation_id]);
};
