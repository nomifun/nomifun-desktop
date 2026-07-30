/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Pure logic for the per-model "advanced" editor: protocol options and the
 * quick-fields (`endpoint` / `request_shape`) ⇄ raw-JSON `params` mapping.
 */

/**
 * Known invoke protocol identifiers (mirror
 * `crates/backend/nomifun-model-invoke/src/routes_table.rs`). Empty string in
 * the UI means "auto: route by task".
 */
export const MODEL_PROTOCOL_OPTIONS = [
  'openai.images',
  'openai.videos',
  'openai.chat_text',
  'openai.embeddings',
  'openai.audio_speech',
  'openai.audio_transcriptions',
  'gemini.generate_content',
  'gemini.generate_text',
  'deepgram.listen',
  'ark.images',
  'ark.video_jobs',
  'volc.asr_file',
] as const;

export const REQUEST_SHAPE_OPTIONS = ['json', 'multipart'] as const;

export interface ModelParamsSplit {
  endpoint: string;
  requestShape: string;
  /** Pretty-printed JSON of the remaining params keys; '' when none. */
  restJson: string;
}

const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

/** Split a row's `params` JSON into quick fields + the rest for the JSON editor. */
export const splitModelParams = (params: unknown): ModelParamsSplit => {
  if (!isPlainObject(params)) return { endpoint: '', requestShape: '', restJson: '' };
  const { endpoint, request_shape: requestShape, ...rest } = params;
  return {
    endpoint: typeof endpoint === 'string' ? endpoint : '',
    requestShape: typeof requestShape === 'string' ? requestShape : '',
    restJson: Object.keys(rest).length > 0 ? JSON.stringify(rest, null, 2) : '',
  };
};

export type ModelParamsMergeResult =
  | { ok: true; params: Record<string, unknown> }
  | { ok: false; error: 'invalid_json' | 'json_not_object' };

/**
 * Merge the quick fields back into the raw-JSON editor content. Quick fields
 * win over same-named keys in the JSON; empty quick fields remove the key.
 * Returns the full object to send as `params` (may be `{}` = no overrides).
 */
export const mergeModelParams = (
  restJson: string,
  endpoint: string,
  requestShape: string
): ModelParamsMergeResult => {
  let rest: unknown = {};
  const raw = restJson.trim();
  if (raw) {
    try {
      rest = JSON.parse(raw);
    } catch {
      return { ok: false, error: 'invalid_json' };
    }
    if (!isPlainObject(rest)) return { ok: false, error: 'json_not_object' };
  }
  const merged: Record<string, unknown> = { ...(rest as Record<string, unknown>) };
  const nextEndpoint = endpoint.trim();
  const nextShape = requestShape.trim();
  if (nextEndpoint) merged.endpoint = nextEndpoint;
  else delete merged.endpoint;
  if (nextShape) merged.request_shape = nextShape;
  else delete merged.request_shape;
  return { ok: true, params: merged };
};
