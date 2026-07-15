/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const ID_PREFIX = /^[a-z][a-z0-9]{0,31}$/;

function randomBytes(): Uint8Array {
  const bytes = new Uint8Array(16);
  try {
    const cryptoObj = globalThis.crypto;
    if (cryptoObj && typeof cryptoObj.getRandomValues === 'function') {
      cryptoObj.getRandomValues(bytes);
      return bytes;
    }
  } catch {
    // Use the non-cryptographic fallback only when WebCrypto is unavailable.
  }
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Math.floor(Math.random() * 256);
  }
  return bytes;
}

/** Generate a canonical RFC 9562 UUIDv7 without a runtime dependency. */
function uuidv7(): string {
  const bytes = randomBytes();
  let timestamp = BigInt(Date.now());
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = Number(timestamp & 0xffn);
    timestamp >>= 8n;
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (value) => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/**
 * Mint a full canonical lowercase UUIDv7.
 *
 * This helper is exported for non-entity values that need UUIDv7 entropy.
 * Persisted entities should use {@link prefixedId} so their namespace remains
 * explicit at every protocol and storage boundary.
 */
export const shortId = (): string => uuidv7();

/**
 * Mint an entity ID in the unified `{registered-prefix}_{UUIDv7}` format.
 *
 * The prefix validation mirrors `nomifun_common::validate_id_prefix`:
 * lowercase ASCII letter first, then lowercase ASCII letters or digits, with
 * a maximum of 32 characters. Invalid programmer-supplied prefixes fail fast
 * instead of minting an ambiguous identifier.
 */
export const prefixedId = (prefix: string): string => {
  if (!ID_PREFIX.test(prefix)) {
    throw new TypeError(`Invalid entity ID prefix: ${prefix}`);
  }
  return `${prefix}_${uuidv7()}`;
};
