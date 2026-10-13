import { normalizeModelFailureDiagnostic, sanitizeDiagnosticDetail, type ModelFailureDiagnostic } from './providerDiagnostic';

export const KNOWLEDGE_WRITEBACK_STAGES = [
  'resolve', 'connect', 'first_frame', 'stream', 'decode', 'validate', 'write', 'settle', 'cancelled', 'interrupted',
] as const;
export type KnowledgeWritebackStage = (typeof KNOWLEDGE_WRITEBACK_STAGES)[number];

/** Canonical observations only; absent/invalid transport values remain unknown. */
export interface KnowledgeWritebackDiagnostic {
  stage: KnowledgeWritebackStage;
  code?: string;
  detail?: string;
  elapsed_ms?: number;
  first_frame_ms?: number;
  received_frames?: number;
  received_bytes?: number;
  output_bytes?: number;
  input_tokens_estimate?: number;
  output_token_limit?: number;
  output_tokens?: number;
  provider_diagnostic?: ModelFailureDiagnostic;
}

export function normalizeKnowledgeWritebackDiagnostic(value: unknown): KnowledgeWritebackDiagnostic | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const data = value as Record<string, unknown>;
  if (!KNOWLEDGE_WRITEBACK_STAGES.includes(data.stage as KnowledgeWritebackStage)) return undefined;
  const result: KnowledgeWritebackDiagnostic = { stage: data.stage as KnowledgeWritebackStage };
  if (typeof data.code === 'string' && /^[A-Z0-9_]{1,96}$/.test(data.code)) result.code = data.code;
  if (typeof data.detail === 'string' && new TextEncoder().encode(data.detail).length <= 2048) {
    const detail = sanitizeDiagnosticDetail(data.detail);
    if (detail) result.detail = detail;
  }
  for (const key of ['elapsed_ms', 'first_frame_ms', 'received_frames', 'received_bytes', 'output_bytes',
    'input_tokens_estimate', 'output_token_limit', 'output_tokens'] as const) {
    if (Number.isSafeInteger(data[key]) && Number(data[key]) >= 0
      && (key !== 'output_token_limit' || Number(data[key]) <= 0xffffffff)) result[key] = data[key] as number;
  }
  if (result.first_frame_ms !== undefined && result.elapsed_ms !== undefined && result.first_frame_ms > result.elapsed_ms) {
    delete result.first_frame_ms;
  }
  const provider = normalizeModelFailureDiagnostic(data.provider_diagnostic);
  if (provider) result.provider_diagnostic = provider;
  return result;
}
