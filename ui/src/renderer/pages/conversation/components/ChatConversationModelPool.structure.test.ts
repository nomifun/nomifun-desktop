import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');

describe('Conversation lead model authority', () => {
  test('updates the lead model and collaboration pool atomically for selection and healing', () => {
    expect(
      source.match(/updates: \{ model: selected, execution_model_pool, execution_template_id: null \}/g),
    ).toHaveLength(2);
    expect(source.includes('updates: { model: selected }')).toBe(false);
  });
});
