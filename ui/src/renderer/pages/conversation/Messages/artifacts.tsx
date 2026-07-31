/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationArtifactId } from '@/common/types/conversationArtifact';
import type { ConversationId } from '@/common/types/ids';

import { ipcBridge } from '@/common';
import type { IConversationArtifact, IConversationArtifactStatus } from '@/common/adapter/ipcBridge';
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';

type ConversationArtifactContextValue = {
  artifacts: IConversationArtifact[];
  upsertArtifact: (artifact: IConversationArtifact) => void;
  updateArtifactStatus: (
    conversation_artifact_id: ConversationArtifactId,
    status: IConversationArtifactStatus
  ) => void;
};

const ConversationArtifactContext = createContext<ConversationArtifactContextValue>({
  artifacts: [],
  upsertArtifact: () => {},
  updateArtifactStatus: () => {},
});

function upsertArtifacts(
  current: IConversationArtifact[],
  next: IConversationArtifact | IConversationArtifact[]
): IConversationArtifact[] {
  const incoming = Array.isArray(next) ? next : [next];
  if (!incoming.length) return current;

  const artifactById = new Map(
    current.map((artifact) => [artifact.conversation_artifact_id, artifact])
  );
  for (const artifact of incoming) {
    artifactById.set(artifact.conversation_artifact_id, artifact);
  }

  return Array.from(artifactById.values()).toSorted((a, b) => a.created_at - b.created_at);
}

export const useConversationArtifacts = (): IConversationArtifact[] =>
  useContext(ConversationArtifactContext).artifacts;

export const useUpdateConversationArtifactStatus = (): ((
  conversation_artifact_id: ConversationArtifactId,
  status: IConversationArtifactStatus
) => void) => useContext(ConversationArtifactContext).updateArtifactStatus;

export const ConversationArtifactProvider: React.FC<React.PropsWithChildren<{ conversation_id: ConversationId }>> = ({
  conversation_id,
  children,
}) => {
  const [artifacts, setArtifacts] = useState<IConversationArtifact[]>([]);

  const upsertArtifact = useCallback((artifact: IConversationArtifact) => {
    setArtifacts((current) => upsertArtifacts(current, artifact));
  }, []);

  const updateArtifactStatus = useCallback(
    (conversation_artifact_id: ConversationArtifactId, status: IConversationArtifactStatus) => {
      setArtifacts((current) =>
        current.map((artifact) =>
          artifact.conversation_artifact_id === conversation_artifact_id
            ? { ...artifact, status, updated_at: Date.now() }
            : artifact
        )
      );
    },
    []
  );

  const loadArtifacts = useCallback(
    (isCurrent: () => boolean) =>
      ipcBridge.conversation.listArtifacts
        .invoke({ conversation_id })
        .then((items) => {
          if (!isCurrent()) return;
          // Merge instead of replace: an artifactStream frame that arrived on
          // the new socket while this GET was in flight is NEWER than the
          // snapshot the GET read — replacing would make its card vanish.
          setArtifacts((current) => upsertArtifacts(current, items));
        })
        .catch((error) => {
          console.error('[ConversationArtifactProvider] Failed to load artifacts:', error);
        }),
    [conversation_id]
  );

  // Initial durable snapshot + gap recovery under one lifecycle: WebSocket
  // delivery has no replay, so any gap (reconnect, server lag resync) may
  // have dropped artifactStream events and must reload the snapshot.
  useEffect(() => {
    let alive = true;
    setArtifacts([]);

    void loadArtifacts(() => alive);
    const offReconnected = ipcBridge.conversation.reconnected.on(() => {
      void loadArtifacts(() => alive);
    });

    return () => {
      alive = false;
      offReconnected();
    };
  }, [loadArtifacts]);

  useEffect(() => {
    if (!conversation_id) return;

    return ipcBridge.conversation.artifactStream.on((artifact: IConversationArtifact) => {
      if (artifact.conversation_id !== conversation_id) return;
      upsertArtifact(artifact);
    });
  }, [conversation_id, upsertArtifact]);

  const value = useMemo<ConversationArtifactContextValue>(
    () => ({
      artifacts,
      upsertArtifact,
      updateArtifactStatus,
    }),
    [artifacts, upsertArtifact, updateArtifactStatus]
  );

  return <ConversationArtifactContext.Provider value={value}>{children}</ConversationArtifactContext.Provider>;
};
