/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { conversationTarget } from '@/common/types/ids';
import type { IDirOrFile, IResponseMessage } from '@/common/adapter/ipcBridge';
import { addEventListener, emitter } from '@/renderer/utils/emitter';
import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';
import { useAbortUploadsOnConversationChange } from '@/renderer/hooks/file/useAbortUploadsOnConversationChange';
import React, { useCallback, useMemo, useRef } from 'react';
import WorkspaceRailBody from './WorkspaceRailBody';
import type { SelectedFile, WorkspaceProps, WorkspaceSource } from './types';

/**
 * Map a source-agnostic {@link SelectedFile} back to the conversation SendBox
 * payload shape ({@link FileOrFolderItem}). `keepEmptyRelativePath=false`
 * reproduces the legacy "append" behavior where an empty relativePath was
 * coerced to `undefined`.
 */
const toFileOrFolderItem = (item: SelectedFile, keepEmptyRelativePath: boolean): FileOrFolderItem => ({
  path: item.fullPath,
  name: item.name,
  isFile: item.isFile,
  relativePath: keepEmptyRelativePath ? item.relativePath : item.relativePath || undefined,
});

/**
 * ChatWorkspace — 会话工作区右栏（会话源绑定）
 *
 * Thin binding that adapts a conversation into a {@link WorkspaceSource} and
 * renders the source-agnostic {@link WorkspaceRailBody}. All conversation-only
 * mechanisms (SendBox file-selection bridge, agent-stream auto-refresh, search
 * stream, upload/paste/drag) are wired here through the source's optional
 * capabilities; the body itself knows nothing about conversations.
 *
 * Behavior is identical to the previous monolithic ChatWorkspace.
 */
const ChatWorkspace: React.FC<WorkspaceProps> = ({
  conversation_id,
  workspace,
  isTemporaryWorkspace: isTemporaryWorkspaceProp,
  eventPrefix = 'acp',
  messageApi,
  extraTabs,
}) => {
  // Bind workspace uploads to the conversation lifecycle: switching the
  // workspace conversation or unmounting the panel cancels in-flight uploads.
  // The upload subsystem keys aborts by string conversation id, so serialize.
  useAbortUploadsOnConversationChange(conversation_id, 'workspace');

  // --- Tree data provider (conversation getWorkspace endpoint) ---------------
  const tree = useMemo(
    () => ({
      key: conversation_id,
      target: conversationTarget(conversation_id),
      listRoot: (search?: string) =>
        ipcBridge.conversation.getWorkspace.invoke({
          conversation_id,
          workspace,
          path: workspace,
          search: search || '',
        }),
      listChildren: (node: { fullPath: string; relativePath: string }) =>
        ipcBridge.conversation.getWorkspace.invoke({
          conversation_id,
          workspace,
          path: node.fullPath,
        }),
    }),
    [conversation_id, workspace]
  );

  // --- Outbound selection → SendBox emitter ----------------------------------
  const onSelectFiles = useCallback(
    (items: SelectedFile[]) => {
      emitter.emit(
        `${eventPrefix}.selected.file`,
        items.map((item) => toFileOrFolderItem(item, true))
      );
    },
    [eventPrefix]
  );

  const onAppendFiles = useCallback(
    (items: SelectedFile[]) => {
      emitter.emit(
        `${eventPrefix}.selected.file.append`,
        items.map((item) => toFileOrFolderItem(item, false))
      );
    },
    [eventPrefix]
  );

  // --- External refresh: agent-stream writes (throttled) + manual refresh ----
  // Throttle state lives across the subscription lifetime, so it is owned here
  // (the source), not in the body. Mirrors the former useWorkspaceEvents logic.
  const throttleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingRef = useRef(false);

  const subscribeRefresh = useCallback(
    (cb: () => void) => {
      const throttledRefresh = () => {
        if (throttleTimerRef.current) {
          pendingRef.current = true; // Mark pending so trailing refresh fires after window
          return;
        }
        cb();
        throttleTimerRef.current = setTimeout(() => {
          throttleTimerRef.current = null;
          if (pendingRef.current) {
            pendingRef.current = false;
            cb(); // Fire trailing refresh for any calls missed during throttle window
          }
        }, 2000);
      };

      const handleResponse = (data: IResponseMessage) => {
        if (data.conversation_id && data.conversation_id !== conversation_id) return;

        if (data.type === 'acp_tool_call') {
          const acpData = data.data as { update?: { kind?: string; status?: string } } | undefined;
          const kind = acpData?.update?.kind;
          const status = acpData?.update?.status;
          const shouldRefresh = kind === 'edit' || kind === 'execute' || (status === 'completed' && kind !== 'read');
          if (shouldRefresh) {
            throttledRefresh();
          }
        }
        if (data.type === 'tool_call') {
          const toolData = data.data as { status?: string } | undefined;
          if (toolData?.status === 'completed') {
            throttledRefresh();
          }
        }
      };

      const unsubscribeStream = ipcBridge.acpConversation.responseStream.on(handleResponse);
      const unsubscribeManual = addEventListener(`${eventPrefix}.workspace.refresh`, () => cb());

      return () => {
        unsubscribeStream();
        unsubscribeManual();
        if (throttleTimerRef.current) {
          clearTimeout(throttleTimerRef.current);
          throttleTimerRef.current = null;
        }
      };
    },
    [conversation_id, eventPrefix]
  );

  // --- Inbound selection sync: SendBox tag close (#1083) + clear -------------
  const subscribeSelectionSync = useCallback(
    (cb: (folders: SelectedFile[]) => void) => {
      // The emitter payload may include bare path strings (FileSelectionItem =
      // string | FileOrFolderItem). Normalize strings to an all-undefined shape
      // so behavior is preserved exactly while access stays type-safe, then keep
      // only folders (non-files) — the same filter the tree applied previously.
      const toFolders = (rawItems: Array<string | FileOrFolderItem>): SelectedFile[] =>
        rawItems
          .map((item): Partial<FileOrFolderItem> => (typeof item === 'string' ? {} : item))
          .filter((item) => !item.isFile)
          .map((item) => ({
            name: item.name ?? '',
            fullPath: item.path ?? '',
            relativePath: item.relativePath ?? '',
            isFile: false,
          }));

      const unsubscribeSync = addEventListener(`${eventPrefix}.selected.file`, (rawItems) => {
        cb(toFolders(rawItems));
      });
      // Clearing selection (after sending a message) is an empty folder set.
      const unsubscribeClear = addEventListener(`${eventPrefix}.selected.file.clear`, () => {
        cb([]);
      });

      return () => {
        unsubscribeSync();
        unsubscribeClear();
      };
    },
    [eventPrefix]
  );

  // --- Streamed search-match replacement -------------------------------------
  // NOTE: `responseSearchWorkSpace` is a stub provider in the HTTP backend (its
  // `.provider` is a no-op returning void), so this channel never fires today —
  // workspace search is fully served by loadWorkspace/onSearch. We preserve the
  // registration exactly (behavior = nothing) and return a no-op unsubscribe.
  const subscribeFileTreeReplace = useCallback((cb: (root: IDirOrFile) => void) => {
    ipcBridge.conversation.responseSearchWorkSpace.provider((data) => {
      if (data.match) cb(data.match);
      return Promise.resolve();
    });
    return () => {};
  }, []);

  // --- Assemble the conversation source --------------------------------------
  const source = useMemo<WorkspaceSource>(
    () => ({
      workspace,
      tree,
      isTemporary: isTemporaryWorkspaceProp ?? false,
      // Intentionally leave `lazyChanges` unset (falsy): conversations init the
      // file-snapshot EAGERLY on mount for parity with pre-rail behavior, so the
      // baseline is captured before any agent edits and snapshot-mode workspaces
      // surface those edits correctly. (Terminal sources opt into laziness.)
      onSelectFiles,
      onAppendFiles,
      subscribeRefresh,
      subscribeSelectionSync,
      subscribeFileTreeReplace,
      upload: { trackingKey: conversation_id },
      extraTabs,
    }),
    [
      workspace,
      tree,
      isTemporaryWorkspaceProp,
      onSelectFiles,
      onAppendFiles,
      subscribeRefresh,
      subscribeSelectionSync,
      subscribeFileTreeReplace,
      conversation_id,
      extraTabs,
    ]
  );

  return <WorkspaceRailBody source={source} messageApi={messageApi} />;
};

export default ChatWorkspace;
