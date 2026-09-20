import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

type Rgb = [number, number, number];

const workspaceCss = readFileSync(
  new URL('./AgentCapabilityWorkspace.module.css', import.meta.url),
  'utf8'
);
const defaultThemeCss = readFileSync(
  new URL('../settings/DisplaySettings/presets/rhythm-dark.css', import.meta.url),
  'utf8'
);

const colorsFor = (token: string): Rgb[] => [...defaultThemeCss.matchAll(
  new RegExp(`--${token}:\\s*#([0-9a-f]{6})`, 'gi')
)].map((match) => [0, 2, 4].map((offset) =>
  Number.parseInt(match[1].slice(offset, offset + 2), 16)
) as Rgb);

const mix = (left: Rgb, right: Rgb): Rgb => left.map((channel, index) =>
  Math.round((channel + right[index]) / 2)
) as Rgb;

const luminance = ([red, green, blue]: Rgb): number => {
  const [r, g, b] = [red, green, blue].map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

const contrastRatio = (foreground: Rgb, background: Rgb): number => {
  const foregroundLuminance = luminance(foreground);
  const backgroundLuminance = luminance(background);
  return (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
    (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
};

const rule = (selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return workspaceCss.match(new RegExp(`${escaped}\\s*\\{([^}]+)\\}`))?.[1] ?? '';
};

describe('Agent capability workspace contrast contract', () => {
  test('keeps muted copy readable on raised cards in both default theme modes', () => {
    const secondary = colorsFor('color-text-2');
    const tertiary = colorsFor('color-text-3');
    const cards = colorsFor('color-bg-5');

    expect(secondary).toHaveLength(2);
    expect(tertiary).toHaveLength(2);
    expect(cards).toHaveLength(2);
    for (let index = 0; index < cards.length; index += 1) {
      expect(contrastRatio(mix(secondary[index], tertiary[index]), cards[index])).toBeGreaterThanOrEqual(4.5);
      expect(contrastRatio(tertiary[index], cards[index])).toBeGreaterThanOrEqual(3);
    }
  });

  test('uses distinct canvas, rail and card surfaces with readable small copy', () => {
    expect(workspaceCss).toContain('--capability-canvas-surface: var(--color-bg-2)');
    expect(workspaceCss).toContain('--capability-rail-surface: var(--color-bg-1)');
    expect(workspaceCss).toContain('--capability-card-surface: var(--color-bg-5');
    expect(workspaceCss).toContain('--capability-muted-text: color-mix(in srgb, var(--color-text-2) 50%, var(--color-text-3))');
    expect(rule('.moduleCard')).toContain('background: var(--capability-card-surface)');
    expect(rule('.dependencyNotice')).toContain('color: var(--color-text-2)');
    expect(rule('.moduleCopy p')).toContain('color: var(--capability-muted-text)');
    expect(rule('.moduleCopy p')).toContain('font-size: 12px');
    expect(rule('.categories small')).toContain('color: var(--capability-muted-text)');
  });

  test('reserves the brand color for the enabled switch state', () => {
    expect(rule('.moduleSwitch')).toContain('border: 1px solid var(--capability-idle-control)');
    expect(rule('.moduleSwitch')).toContain('background: var(--capability-card-surface)');
    expect(rule(".moduleSwitch[aria-checked='true']")).toContain('background: rgb(var(--primary-6))');
  });
});
