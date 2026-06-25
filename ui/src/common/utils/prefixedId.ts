/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Lowercase Crockford-style base32 alphabet (drops i/l/o/u for legibility),
 * in ascending ASCII order so a fixed-width big-endian encoding sorts
 * lexicographically by the integer it encodes. MUST match
 * `SHORT_ID_ALPHABET` in the Rust `nomifun-common::id` module.
 */
const SHORT_ID_ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz';

/** Base32 char counts for the 45-bit timestamp and 35-bit random components. */
const TIME_CHARS = 9;
const RAND_CHARS = 7;

/**
 * Encode the low `chars * 5` bits of `value` as `chars` base32 characters,
 * most-significant character first (big-endian), so the text sorts like the
 * value. Mirrors `encode_base32` on the Rust side.
 */
const encodeBase32 = (value: bigint, chars: number): string => {
  let v = value;
  const out = new Array<string>(chars);
  for (let i = chars - 1; i >= 0; i--) {
    out[i] = SHORT_ID_ALPHABET[Number(v & 31n)];
    v >>= 5n;
  }
  return out.join('');
};

/**
 * Self-contained sortable short-id generator: a 45-bit unix-millisecond
 * timestamp (9 base32 chars) followed by 35 random bits (7 base32 chars),
 * for a 16-char body. Mirrors the Rust side
 * (`nomifun-common::generate_prefixed_id`), so ids minted by either side
 * interleave and sort identically — they are lexicographically time-ordered
 * and globally unique, but roughly half the length of the former UUIDv7 tail.
 *
 * Uses crypto.getRandomValues when available; falls back to Math.random
 * (format preserved, randomness not cryptographically secure).
 */
export const shortId = (): string => {
  const ms = BigInt(Date.now()) & ((1n << 45n) - 1n);

  const words = new Uint32Array(2);
  let filled = false;
  try {
    const cryptoObj = globalThis.crypto;
    if (cryptoObj && typeof cryptoObj.getRandomValues === 'function') {
      cryptoObj.getRandomValues(words);
      filled = true;
    }
  } catch {
    // fall through to Math.random
  }
  if (!filled) {
    words[0] = Math.floor(Math.random() * 2 ** 32) >>> 0;
    words[1] = Math.floor(Math.random() * 2 ** 32) >>> 0;
  }
  const rand = ((BigInt(words[0]) << 32n) | BigInt(words[1])) & ((1n << 35n) - 1n);

  return encodeBase32(ms, TIME_CHARS) + encodeBase32(rand, RAND_CHARS);
};

/**
 * Mint an entity ID in the unified `{prefix}_{shortId}` format, e.g.
 * `prefixedId('msg')` -> `msg_0fh3k…`. Frontend mirror of the Rust
 * `nomifun-common::generate_prefixed_id` — the minting convention for the
 * TEXT short-id entities (messages `msg_`, providers `prov_`, …).
 *
 * NOTE: conversations/requirements/terminal sessions are now
 * `INTEGER PRIMARY KEY AUTOINCREMENT` and are minted **only** by the backend
 * (`last_insert_rowid()`); the frontend must NOT mint `conv_`/`req_`/`term_`
 * ids — create the row first and use the integer id the backend returns. See
 * the numeric-id spec §5 (create flow). Use this for any TEXT id that is sent
 * to / stored by the backend; for throwaway local UI keys, `uuid()` from
 * ./utils is fine.
 */
export const prefixedId = (prefix: string): string => `${prefix}_${shortId()}`;
