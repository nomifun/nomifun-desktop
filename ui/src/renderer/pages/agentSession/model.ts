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

export const inlinePayload = (payload: unknown): unknown => {
  if (!payload || typeof payload !== 'object') return payload;
  const record = payload as Record<string, unknown>;
  return record.encoding === 'inline_json' ? record.value : payload;
};

const latestPayload = (document: IAgentSessionProjectionDocument): unknown =>
  inlinePayload(document.events.at(-1)?.payload);

export function projectionCard(projection: IAgentSessionMessageProjection): SessionCardModel {
  const document = projection.projection;
  const latestKind = document.events.at(-1)?.kind ?? projection.presentation_intent;
  const payload = latestPayload(document);
  if (projection.presentation_intent === 'message') {
    const user = document.events.some((event) => event.kind === 'message/user-accepted');
    const record = payload && typeof payload === 'object' ? (payload as Record<string, unknown>) : null;
    return {
      id: projection.projection_id,
      kind: 'message',
      role: user ? 'user' : 'assistant',
      state: document.state,
      title: user ? 'user' : 'assistant',
      content:
        document.content ??
        (typeof record?.content === 'string' ? record.content : undefined),
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  if (projection.presentation_intent === 'tool') {
    const record = payload && typeof payload === 'object' ? (payload as Record<string, unknown>) : null;
    return {
      id: projection.projection_id,
      kind: 'tool',
      state: document.state,
      title:
        (typeof record?.action_id === 'string' && record.action_id) ||
        (typeof record?.capability_id === 'string' && record.capability_id) ||
        latestKind,
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
      title: latestKind,
      details: payload,
      firstSeq: projection.first_seq,
      lastSeq: projection.last_seq,
    };
  }
  return {
    id: projection.projection_id,
    kind: 'status',
    state: document.state,
    title: latestKind,
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
