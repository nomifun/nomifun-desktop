export type SessionReasoningEffort = 'low' | 'medium' | 'high';

export const SESSION_REASONING_EFFORTS = ['low', 'medium', 'high'] as const;

const REASONING_EFFORT_PROTOCOLS = new Set([
  'openai.chat_text',
  'openai.responses',
  'gemini.generate_text',
]);

export const protocolSupportsReasoningEffort = (protocol: string | undefined): boolean =>
  Boolean(protocol && REASONING_EFFORT_PROTOCOLS.has(protocol.trim()));

export const isSessionReasoningEffort = (value: unknown): value is SessionReasoningEffort =>
  typeof value === 'string' && SESSION_REASONING_EFFORTS.includes(value as SessionReasoningEffort);
