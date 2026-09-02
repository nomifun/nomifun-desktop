import type {
  IAgentSessionMessageProjection,
  IAgentSessionProjectionDocument,
} from '@/common/adapter/ipcBridge';

export type SessionCardKind = 'message' | 'tool' | 'effect' | 'status';

export interface SessionCardModel {
  id: string;
  kind: SessionCardKind;
  role?: 'user' | 'assistant';
  state?: string;
  title: string;
  content?: string;
  details?: unknown;
  firstSeq: number;
  lastSeq: number;
}

type LegacyProjectionEvent = {
  kind: string;
  payload: unknown;
};

type ProjectionDocument = Omit<
  IAgentSessionProjectionDocument,
  'events'
> & {
  events?: LegacyProjectionEvent[];
  tool_summary?: unknown;
  reference?: unknown;
  terminal_effect?: unknown;
};

export const inlinePayload = (payload: unknown): unknown => {
  if (!payload || typeof payload !== 'object') return payload;
  const record = payload as Record<string, unknown>;
  return record.encoding === 'inline_json' ? record.value : payload;
};

const asRecord = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;

const legacyEvents = (document: ProjectionDocument): LegacyProjectionEvent[] =>
  Array.isArray(document.events) ? document.events : [];

const legacyLatestPayload = (document: ProjectionDocument): unknown =>
  inlinePayload(legacyEvents(document).at(-1)?.payload);

const legacyMessageContent = (document: ProjectionDocument): string | undefined => {
  let content: string | undefined;
  for (const event of legacyEvents(document)) {
    const payload = asRecord(inlinePayload(event.payload));
    if (
      (event.kind === 'message/user-accepted' || event.kind === 'message/content-part') &&
      typeof payload?.content === 'string'
    ) {
      content =
        event.kind === 'message/user-accepted'
          ? payload.content
          : `${content ?? ''}${payload.content}`;
    }
  }
  return content;
};

const firstSummaryString = (
  summary: unknown,
  keys: string[]
): string | undefined => {
  const record = asRecord(summary);
  for (const key of keys) {
    if (typeof record?.[key] === 'string' && record[key]) return record[key] as string;
  }
  return undefined;
};

const latestKind = (document: ProjectionDocument, fallback: string): string =>
  legacyEvents(document).at(-1)?.kind ?? document.state ?? fallback;

const projectionDetails = (
  document: ProjectionDocument,
  intent: string
): unknown => {
  const summary =
    intent === 'tool'
      ? document.tool_summary
      : intent === 'effect'
        ? document.terminal_effect
        : document.reference;
  return summary ?? legacyLatestPayload(document);
};

export function projectionCard(projection: IAgentSessionMessageProjection): SessionCardModel {
  // The IPC interface still accepts the legacy shape, while new projections
  // intentionally omit events and expose bounded summaries instead.
  const document = projection.projection as unknown as ProjectionDocument;
  const intent = projection.presentation_intent;
  const payload = projectionDetails(document, intent);
  const payloadRecord = asRecord(payload);
  if (projection.presentation_intent === 'message') {
    const user =
      document.state === 'accepted' ||
      legacyEvents(document).some((event) => event.kind === 'message/user-accepted');
    return {
      id: projection.projection_id,
      kind: 'message',
      role: user ? 'user' : 'assistant',
      state: document.state,
      title: user ? 'user' : 'assistant',
      content:
        document.content ??
        (typeof payloadRecord?.content === 'string'
          ? payloadRecord.content
          : legacyMessageContent(document)),
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  if (projection.presentation_intent === 'tool') {
    return {
      id: projection.projection_id,
      kind: 'tool',
      state: document.state,
      title:
        firstSummaryString(document.tool_summary, [
          'action_id',
          'capability_id',
          'name',
          'tool',
        ]) ?? latestKind(document, intent),
      details: payload,
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  if (projection.presentation_intent === 'effect') {
    return {
      id: projection.projection_id,
      kind: 'effect',
      state: document.state,
      title:
        firstSummaryString(document.terminal_effect, [
          'action_id',
          'capability_id',
          'effect',
        ]) ?? latestKind(document, intent),
      details: payload,
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  return {
    id: projection.projection_id,
    kind: 'status',
    state: document.state,
    title: latestKind(document, intent),
    details: payload,
    firstSeq: projection.first_seq,
    lastSeq: projection.last_seq,
  };
}

export const projectionCards = (
  projections: IAgentSessionMessageProjection[]
): SessionCardModel[] =>
  projections
    .map(projectionCard)
    .sort((left, right) => left.firstSeq - right.firstSeq);

export const jsonDetails = (value: unknown): string =>
  value == null ? '' : JSON.stringify(value, null, 2);
