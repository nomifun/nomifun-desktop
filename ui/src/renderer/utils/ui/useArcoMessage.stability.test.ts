import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useArcoMessage.ts', import.meta.url), 'utf8');

describe('useArcoMessage', () => {
  test('keeps one Arco message instance for the lifetime of a rendered component', () => {
    expect(source.includes('const latest = useRef(message);')).toBe(true);
    expect(source.includes('latest.current = message;')).toBe(false);
    expect(source.includes('Reflect.get(latest.current as object, prop, receiver)')).toBe(true);
  });
});
