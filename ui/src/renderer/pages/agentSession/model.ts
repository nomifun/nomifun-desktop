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
  detailText?: string;
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

const INTERNAL_UUID_PATTERN =
  /\b[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/i;
const INTERNAL_DIGEST_PATTERN = /\b[a-f0-9]{64}\b/i;

const containsInternalValue = (value: string): boolean =>
  INTERNAL_UUID_PATTERN.test(value) ||
  INTERNAL_DIGEST_PATTERN.test(value) ||
  /[{}\[\]"]/.test(value);

const humanizeLabel = (value: unknown, fallback: string): string => {
  if (typeof value !== 'string' || !value.trim()) return fallback;
  const normalized = value.trim();
  if (containsInternalValue(normalized)) return fallback;
  const label = normalized
    .replace(/^message[/:_-]*/i, '')
    .replace(/[._/:_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (!label) return fallback;
  return label.length > 80 ? `${label.slice(0, 77)}...` : label;
};

const safeDetailText = (value: unknown, intent: string): string | undefined => {
  const record = asRecord(inlinePayload(value));
  if (!record) {
    if (typeof value !== 'string' || containsInternalValue(value)) return undefined;
    return value.trim() || undefined;
  }

  const summary = firstSummaryString(record, ['summary', 'message', 'description']);
  if (summary && !containsInternalValue(summary)) {
    return summary.length > 160 ? `${summary.slice(0, 157)}...` : summary;
  }
  const state = firstSummaryString(record, ['result_state', 'state', 'status']);
  if (state) {
    return `Status: ${humanizeLabel(state, 'Recorded')}`;
  }
  if (intent === 'effect') return 'Effect recorded';
  if (intent === 'tool') return 'Tool action recorded';
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
      title: user ? 'User message' : 'Agent message',
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
      title: humanizeLabel(
        firstSummaryString(document.tool_summary, [
          'action_id',
          'capability_id',
          'name',
          'tool',
        ]) ?? latestKind(document, intent),
        'Tool action'
      ),
      details: payload,
      detailText: safeDetailText(payload, intent),
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  if (projection.presentation_intent === 'effect') {
    return {
      id: projection.projection_id,
      kind: 'effect',
      state: document.state,
      title: humanizeLabel(
        firstSummaryString(document.terminal_effect, [
          'action_id',
          'capability_id',
          'effect',
        ]) ?? latestKind(document, intent),
        'Effect'
      ),
      details: payload,
      detailText: safeDetailText(payload, intent),
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  return {
    id: projection.projection_id,
    kind: 'status',
    state: document.state,
    title: humanizeLabel(latestKind(document, intent), 'Session update'),
    details: payload,
    detailText: safeDetailText(payload, intent),
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

/**
 * Compatibility name retained for callers/tests from the first UI slice.
 * It now returns a short, non-JSON summary so a projection can never dump
 * internal payloads into the normal Session transcript.
 */
export const jsonDetails = (value: unknown): string =>
  safeDetailText(value, 'status') ?? '';
