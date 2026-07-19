/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { OpenAI2GeminiConverter } from './OpenAI2GeminiConverter';

describe('Gemini image response completion semantics', () => {
  test('never invents a success message when the provider returns no image or text', () => {
    const converter = new OpenAI2GeminiConverter();
    const response = converter.convertResponse(
      { candidates: [{ content: { parts: [] }, finishReason: 'STOP' }] },
      'gemini-image'
    );

    expect(response.choices[0]?.message.content).toBe('');
    expect(response.choices[0]?.message.images).toBeUndefined();
  });
});

