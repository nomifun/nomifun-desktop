/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  relabelCreativeCanvasPromptMentions,
  type CreativeCanvasPromptMentionBinding,
  type CreativeCanvasPromptReferenceOption,
  type CreativeCanvasReferencePromptChange,
} from './CreativeCanvasReferencePromptInput';

/** Shared draft hydration keeps node-bound mentions stable across graph renders. */
export function useCanvasReferencePromptDraft(
  nodeId: string,
  initialPrompt: string,
  initialMentions: readonly CreativeCanvasPromptMentionBinding[],
  references: readonly CreativeCanvasPromptReferenceOption[],
  onPromptChange?: (change: CreativeCanvasReferencePromptChange) => void
) {
  const { t, i18n } = useTranslation();
  const referenceMentionLabel = useCallback(
    (ordinal: number) =>
      t('creativeStudio.canvas.image.referenceMentionLabel', {
        index: ordinal,
        defaultValue: `图片${ordinal}` as const,
      }),
    [t]
  );
  const referenceAliasSignature = `${i18n.resolvedLanguage ?? i18n.language}:${references
    .map(
      (reference) =>
        `${reference.nodeId}:${reference.ordinal}:${reference.mentionLabel ?? ''}:${reference.disabledReason ? 'disabled' : 'enabled'}`
    )
    .join(',')}`;
  const normalizedInitialDraft = useMemo(
    () =>
      relabelCreativeCanvasPromptMentions(
        initialPrompt,
        initialMentions,
        references,
        referenceMentionLabel
      ),
    // Reference display names and thumbnails do not affect prompt aliases.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [initialMentions, initialPrompt, referenceAliasSignature, referenceMentionLabel]
  );
  // The route clones mention arrays on every canvas render. Object identity is
  // not a new draft: hydrating it again can overwrite a newer native edit.
  const hydratedDraftRef = useRef<{
    nodeId: string;
    draft: CreativeCanvasReferencePromptChange;
  } | null>(null);
  const [prompt, setPrompt] = useState(normalizedInitialDraft.value);
  const [mentions, setMentions] = useState<CreativeCanvasPromptMentionBinding[]>(
    () => structuredClone(normalizedInitialDraft.mentions)
  );

  useEffect(() => {
    const previous = hydratedDraftRef.current;
    if (
      previous?.nodeId === nodeId &&
      previous.draft.value === normalizedInitialDraft.value &&
      previous.draft.mentions.length === normalizedInitialDraft.mentions.length &&
      previous.draft.mentions.every((mention, index) => {
        const next = normalizedInitialDraft.mentions[index]!;
        return mention.id === next.id && mention.sourceNodeId === next.sourceNodeId &&
          mention.fallbackLabel === next.fallbackLabel &&
          mention.start === next.start && mention.end === next.end;
      })
    ) return;
    hydratedDraftRef.current = { nodeId, draft: normalizedInitialDraft };
    setPrompt(normalizedInitialDraft.value);
    setMentions(structuredClone(normalizedInitialDraft.mentions));
    if (normalizedInitialDraft.value !== initialPrompt) {
      onPromptChange?.({
        value: normalizedInitialDraft.value,
        mentions: structuredClone(normalizedInitialDraft.mentions),
      });
    }
    // onPromptChange is an inline route callback; the normalized value is the
    // idempotency boundary that prevents a migration loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialPrompt, nodeId, normalizedInitialDraft]);

  const labels = {
    input: t('creativeStudio.canvas.image.promptLabel', {
      defaultValue: '图片创作提示词',
    }),
    insertReference: t('creativeStudio.canvas.image.insertReference', {
      defaultValue: '引用已连接素材',
    }),
    connectedReferences: t('creativeStudio.canvas.image.connectedReferences', {
      defaultValue: '已连接参考',
    }),
    emptyReferences: t('creativeStudio.canvas.image.noMatchingReferences', {
      defaultValue: '没有匹配的已连接素材',
    }),
    disconnectedReference: t('creativeStudio.canvas.image.referenceDisconnected', {
      defaultValue: '引用已断开',
    }),
    referenceMentionLabel,
  };
  const change = (draft: CreativeCanvasReferencePromptChange): void => {
    setPrompt(draft.value);
    setMentions(draft.mentions);
    onPromptChange?.(draft);
  };
  const clear = (): void => { setPrompt(''); setMentions([]); };
  return { prompt, mentions, labels, change, clear };
}
