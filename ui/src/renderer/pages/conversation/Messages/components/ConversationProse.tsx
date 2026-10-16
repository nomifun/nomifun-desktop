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
  return <StreamingProse {...markdownProps}
    identity={identity ?? `unclassified:${message.id}`}
    text={text}
    sourceText={sourceText}
    sourceOffset={sourceOffset}
    animate={Boolean(identity && exactRange && ingress.isLive(identity, presentation?.liveGeneration))}
    immediate={Boolean(!identity || !exactRange || message.content.replace || interrupted || paused)} />;
}
