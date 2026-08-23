import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./MessageThinking.tsx', import.meta.url), 'utf8');
const sharedSource = readFileSync(
  new URL('../../../../components/chat/ThinkingProcessDisplay.tsx', import.meta.url),
  'utf8'
);
const cssSource = readFileSync(
  new URL('../../../../components/chat/ThinkingProcessDisplay.module.css', import.meta.url),
  'utf8'
);

describe('MessageThinking expansion', () => {
  test('allows a closed process to override a stale streaming status', () => {
    expect(source.includes('completed?: boolean')).toBe(true);
    expect(source.includes("const isDone = completed === true || status === 'done';")).toBe(true);
    expect(source.includes("state={isDone ? 'completed' : 'running'}")).toBe(true);
  });

  test('collapses completed process thinking by default while keeping live thinking open', () => {
    expect(sharedSource.includes("const defaultExpanded = expanded ?? (isProcessVariant ? !isDone : true);")).toBe(true);
    expect(sharedSource.includes('useState(() => defaultExpanded)')).toBe(true);
    expect(sharedSource.includes('onExpandedChange?.(nextExpanded)')).toBe(true);
    expect(sharedSource.includes('useState(!isDone)')).toBe(false);
    expect(sharedSource.includes('setExpanded(false)')).toBe(false);
  });

  test('supports a neutral process timeline variant', () => {
    expect(source.includes("variant = 'standalone'")).toBe(true);
    expect(source.includes('variant={variant}')).toBe(true);
    expect(sharedSource.includes('styles.containerProcess')).toBe(true);
    expect(sharedSource.includes('styles.bodyProcess')).toBe(true);
    expect(cssSource.includes('.containerProcess')).toBe(true);
    expect(cssSource.includes('.bodyProcess')).toBe(true);
    expect(cssSource.includes('background: transparent')).toBe(true);
    expect(cssSource.includes('font-size: var(--conversation-message-font-size')).toBe(true);
  });

  test('frames thinking content with a light thin border', () => {
    expect(cssSource.includes('border: 1px solid var(--color-border-2')).toBe(true);
    expect(cssSource.includes('border-radius: 6px')).toBe(true);
  });

  test('delegates transport-neutral presentation to the shared display', () => {
    expect(
      source.includes(
        "import ThinkingProcessDisplay from '@renderer/components/chat/ThinkingProcessDisplay';"
      )
    ).toBe(true);
    expect(source.includes('content={text}')).toBe(true);
    expect(source.includes('identityKey={message.msg_id ?? message.id}')).toBe(true);
    expect(sharedSource.includes('Transport-specific events stay outside this component.')).toBe(
      true
    );
  });
});
