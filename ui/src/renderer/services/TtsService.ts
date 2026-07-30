/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { buildBackendAuthHeaders, getBaseUrl } from '@/common/adapter/httpBridge';
import type { ProviderId } from '@/common/types/ids';

/**
 * Input for one `/api/tts` synthesis call. Mirrors the wire body
 * `TtsApiRequest` (crates/backend/nomifun-api-types/src/shell.rs — keep in
 * sync): the renderer-facing `providerId` is renamed to `provider_id` on the
 * wire; `voice`/`format` are optional passthroughs to the provider adapter.
 */
export interface TtsSynthesisRequest {
  providerId: ProviderId;
  model: string;
  text: string;
  voice?: string;
  format?: string;
}

export interface TtsSynthesisResult {
  /** Synthesized audio bytes. */
  blob: Blob;
  /** MIME type reported by the backend (`Content-Type`), e.g. `audio/mpeg`. */
  mime: string;
}

/** Mirrors the backend's own `/api/tts` input cap (OpenAI `/audio/speech`). */
const MAX_TTS_TEXT_CHARS = 4096;

const DEFAULT_AUDIO_MIME = 'audio/mpeg';

/**
 * Extract a human-usable detail out of the standard `AppError` JSON envelope
 * (`{ success: false, code, error, details? }`); older callers may also see a
 * `msg` field, so accept both — same tolerance as SpeechToTextService.
 */
const parseErrorDetail = (rawText: string): string => {
  try {
    const payload = JSON.parse(rawText) as { code?: string; error?: string; msg?: string };
    return [payload.code, payload.error || payload.msg].filter(Boolean).join(': ');
  } catch {
    return rawText.trim();
  }
};

/**
 * Synthesize speech via `POST /api/tts`.
 *
 * Unlike the `ApiResponse`-enveloped bridge endpoints this is a BINARY
 * endpoint: a successful synthesis answers `200` with the audio bytes and the
 * asset's MIME as `Content-Type`; errors ride the standard JSON error
 * envelope. Auth/base-url handling mirrors `SpeechToTextService`: requests
 * are sent without explicit credential mode (the trusted desktop backend
 * authenticates via `buildBackendAuthHeaders`, WebUI is same-origin).
 */
export async function synthesizeSpeech(request: TtsSynthesisRequest): Promise<TtsSynthesisResult> {
  const text = request.text;
  if (!text.trim()) {
    throw new Error('TTS_EMPTY_TEXT');
  }
  if ([...text].length > MAX_TTS_TEXT_CHARS) {
    throw new Error('TTS_TEXT_TOO_LONG');
  }

  let response: Response;
  try {
    response = await fetch(`${getBaseUrl()}/api/tts`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...buildBackendAuthHeaders('POST'),
      },
      // `TtsApiRequest` is deny_unknown_fields; `JSON.stringify` drops
      // `undefined` members, so absent voice/format stay off the wire.
      body: JSON.stringify({
        provider_id: request.providerId,
        model: request.model,
        text,
        voice: request.voice,
        format: request.format,
      }),
    });
  } catch {
    throw new Error('TTS_NETWORK_ERROR');
  }

  if (!response.ok) {
    const detail = parseErrorDetail(await response.text());
    throw new Error(`TTS_REQUEST_FAILED:${detail || `${response.status} ${response.statusText}`}`);
  }

  const blob = await response.blob();
  const contentType = response.headers.get('Content-Type');
  // Strip any media-type parameters (`; charset=...`); fall back to the
  // backend's own default when the header is missing.
  const mime = contentType?.split(';')[0]?.trim() || blob.type || DEFAULT_AUDIO_MIME;
  return { blob, mime };
}
