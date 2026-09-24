export type SessionReasoningEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';

export const SESSION_REASONING_EFFORTS = [
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
  'ultra',
] as const;

const STANDARD_REASONING_EFFORTS = ['low', 'medium', 'high'] as const;
const NO_REASONING_EFFORTS: readonly SessionReasoningEffort[] = [];

/**
 * Exact protocol-level envelope. OpenAI-compatible transports retain Ultra as
 * a compatibility extension for providers that expose a tier beyond Max.
 * Individual model APIs may still support only a subset.
 */
export const reasoningEffortsForProtocol = (
  protocol: string | undefined
): readonly SessionReasoningEffort[] => {
  switch (protocol?.trim()) {
    case 'openai.chat_text':
    case 'openai.responses':
      return SESSION_REASONING_EFFORTS;
    case 'gemini.generate_text':
      return STANDARD_REASONING_EFFORTS;
    default:
      return NO_REASONING_EFFORTS;
  }
};

export const protocolSupportsReasoningEffort = (protocol: string | undefined): boolean =>
  reasoningEffortsForProtocol(protocol).length > 0;

export const isSessionReasoningEffort = (value: unknown): value is SessionReasoningEffort =>
  typeof value === 'string' && SESSION_REASONING_EFFORTS.includes(value as SessionReasoningEffort);
