/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** UTF-8 ⇄ base64 helpers for terminal PTY payloads (binary-safe over JSON). */

export function encodeStringToBase64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = '';
  bytes.forEach((b) => {
    binary += String.fromCharCode(b);
  });
  return btoa(binary);
}

export function decodeBase64ToString(b64: string): string {
  const binary = atob(b64);
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/**
 * A per-session decoder for the live PTY stream. The backend reads fixed-size
 * byte buffers that can split a multibyte UTF-8 character across two WebSocket
 * chunks; a fresh `TextDecoder` per chunk would emit U+FFFD at the boundary
 * (this is the macOS "scramble", since macOS PTYs produce smaller, more frequent
 * reads than Windows ConPTY). A single decoder with `{ stream: true }` buffers
 * the incomplete trailing bytes until the next chunk completes the character.
 *
 * Each terminal session MUST own its own decoder instance (state is per-stream).
 */
export function createStreamingDecoder(): (b64: string) => string {
  const decoder = new TextDecoder('utf-8');
  return (b64: string): string => {
    const binary = atob(b64);
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    return decoder.decode(bytes, { stream: true });
  };
}
