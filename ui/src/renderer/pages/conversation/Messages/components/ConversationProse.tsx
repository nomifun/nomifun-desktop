/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageText } from '@/common/chat/chatLib';
import type MarkdownView from '@/renderer/components/Markdown';
import React, { createContext, useContext } from 'react';
import { proseSourceIdentity } from '../proseSource';
import { StreamingProse, useStreamingProseIngress } from '../streamingProse';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';

/** These are the existing Turn presentation gates, never another lifecycle. */
export const ConversationProseContext = createContext<{
  sources: ReadonlyMap<string, string>;
  turns: ReadonlyMap<string, { running: boolean; state: TurnDisclosureProcessState }>;
  pausedTurnId?: string;
  liveGeneration?: number;
} | undefined>(undefined);

type MarkdownProps = Omit<React.ComponentProps<typeof MarkdownView>, 'children' | 'revealOffset'>;

/** Public result membership owns a whole Message. Its canonical ranges still
 * own Markdown boundaries; only an unfiltered complete source can use them. */
function publicNarrationRanges(message: IMessageText, text: string, sourceText: string) {
  const timeline = message.content.timeline;
  if (!timeline || message.content.content !== sourceText
    || timeline.segments.some(item => item.segment_id === message.id)
    || (text !== sourceText && text !== sourceText.trim())) return undefined;
  const narration = timeline.segments.filter(item => item.kind === 'narration');
  if (narration.length < 2) return undefined;
  const textStart = text === sourceText ? 0 : sourceText.length - sourceText.trimStart().length;
  const textEnd = textStart + text.length;
  const ranges: Array<{ id: string; text: string; start: number }> = [];
  let previousEnd = 0;
  for (const [index, item] of narration.entries()) {
    const start = item.start_utf16;
    const end = item.end_utf16;
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start !== previousEnd || end! < start!) return undefined;
    previousEnd = end!;
    const availableEnd = index === narration.length - 1 && item.state === 'running'
      ? Math.max(end!, sourceText.length) : end!;
    const visibleStart = Math.max(start!, textStart);
    const visibleEnd = Math.min(availableEnd, textEnd);
    if (visibleEnd > visibleStart) ranges.push({ id: item.segment_id,
      text: sourceText.slice(visibleStart, visibleEnd), start: visibleStart });
  }
  if (previousEnd < sourceText.length && narration.at(-1)?.state !== 'running') return undefined;
  return ranges;
}

/** One prose path for process narration and public answers. Copy, membership
 * and task settlement continue to read the complete canonical message. */
export default function ConversationProse({ message, text, ...markdownProps }: {
  message: IMessageText;
  text: string;
} & MarkdownProps) {
  const ingress = useStreamingProseIngress();
  const presentation = useContext(ConversationProseContext);
  const identity = proseSourceIdentity(message);
  const sourceText = (identity ? presentation?.sources.get(identity) : undefined) ?? message.content.content;
  const segment = message.content.timeline?.segments.find(item => item.segment_id === message.id
    && item.kind === 'narration');
  const start = segment?.start_utf16 ?? 0;
  const leadingWhitespace = message.content.content.length - message.content.content.trimStart().length;
  const sourceOffset = start + leadingWhitespace;
  // Filters can remove private/protocol bodies. Only exact canonical ranges
  // share a reveal cursor; transformed text is displayed immediately and never
  // guesses an offset into an unrelated visible substring.
  const exactRange = sourceText.slice(sourceOffset, sourceOffset + text.length) === text;
  const gate = message.turn_id ? presentation?.turns.get(message.turn_id) : undefined;
  const interrupted = gate && !gate.running && (gate.state === 'failed' || gate.state === 'canceled');
  const paused = message.turn_id !== undefined && presentation?.pausedTurnId === message.turn_id;
  const ranges = publicNarrationRanges(message, text, sourceText);
  if (ranges) return <>{ranges.map(range => <StreamingProse {...markdownProps} key={range.id}
    identity={identity ?? `unclassified:${message.id}`}
    text={range.text} sourceText={sourceText} sourceOffset={range.start}
    animate={Boolean(identity && ingress.isLive(identity, presentation?.liveGeneration))}
    immediate={Boolean(!identity || message.content.replace || interrupted || paused)} />)}</>;
  return <StreamingProse {...markdownProps}
    identity={identity ?? `unclassified:${message.id}`}
    text={text}
    sourceText={sourceText}
    sourceOffset={sourceOffset}
    animate={Boolean(identity && exactRange && ingress.isLive(identity, presentation?.liveGeneration))}
    immediate={Boolean(!identity || !exactRange || message.content.replace || interrupted || paused)} />;
}
