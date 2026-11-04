import { expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider } from 'react-i18next';
import { createGenerator } from 'unocss';
import unoConfig from '../../../uno.config';
import PreviewToolbar from '../pages/conversation/Preview/components/PreviewPanel/PreviewToolbar';
import { PRESET_THEMES } from '../pages/settings/DisplaySettings/presets';

type Rgb = [number, number, number];

function themeModes(css: string): Map<string, string>[] {
  const modes = [new Map<string, string>(), new Map<string, string>()];
  for (const [, selectors, body] of css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const names = selectors.split(',').map((selector) => selector.trim());
    const mode = names.includes(':root') ? 0 : names.includes("[data-theme='dark']") ? 1 : -1;
    if (mode < 0) continue;
    for (const [, name, value] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) modes[mode].set(name, value.trim());
  }
  return modes;
}

function color(raw: string, variables: Map<string, string>): Rgb {
  const variable = /^var\((--[\w-]+)\)$/.exec(raw);
  if (variable) return color(variables.get(variable[1]) ?? '', variables);
  const hex = /^#([\da-f]{6})$/i.exec(raw);
  if (!hex) throw new Error(`Cannot resolve toolbar color: ${raw}`);
  return [0, 2, 4].map((offset) => parseInt(hex[1].slice(offset, offset + 2), 16)) as Rgb;
}

function contrast(left: Rgb, right: Rgb): number {
  const luminance = (rgb: Rgb) => rgb.reduce((sum, channel, index) => {
    const value = channel / 255;
    const linear = value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    return sum + linear * [0.2126, 0.7152, 0.0722][index];
  }, 0);
  const a = luminance(left), b = luminance(right);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

function toolbarColors(css: string, hovered: boolean): Map<string, string> {
  const styles = new Map<string, { value: string; priority: number }>();
  for (const [, selector, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const hoverRule = selector.includes(':hover');
    if (hoverRule && !hovered) continue;
    for (const [, property, raw] of body.matchAll(/(?:^|;)\s*(color|background-color)\s*:\s*([^;]+)/g)) {
      // Generated utilities have one class; :hover and !important determine
      // which foreground/background wins when the selected control is hovered.
      const priority = (raw.includes('!important') ? 100 : 0) + (hoverRule ? 1 : 0);
      if ((styles.get(property)?.priority ?? -1) > priority) continue;
      styles.set(property, { value: raw.replace(/\s*!important/g, '').trim(), priority });
    }
  }
  return new Map([...styles].map(([property, { value }]) => [property, value]));
}

test('selected preview toolbar text stays readable before and during hover in both preset modes', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: { preview: { exitEdit: 'Exit edit' } } } }, initImmediate: false });
  const noop = () => {};
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><PreviewToolbar
    content_type='code' isMarkdown={false} isHTML={false} isEditable isEditMode
    viewMode='source' isSplitScreenEnabled={false} showOpenInSystemButton={false}
    historyTarget={null} snapshotSaving={false} onViewModeChange={noop}
    onSplitScreenToggle={noop} onEditClick={noop} onExitEdit={noop}
    onSaveSnapshot={noop} onRefreshHistory={noop} renderHistoryDropdown={() => null}
    onOpenInSystem={noop} onDownload={noop}
  /></I18nextProvider>);
  const button = html.match(/<div[^>]*title="Exit edit"[^>]*>/)?.[0];
  const classes = button?.match(/class="([^"]+)"/)?.[1];
  expect(classes).toBeTruthy();
  const uno = await createGenerator(unoConfig);
  const { css } = await uno.generate(classes!, { preflights: false });
  for (const theme of PRESET_THEMES) {
    for (const [mode, variables] of themeModes(theme.css ?? '').entries()) {
      for (const hovered of [false, true]) {
        const styles = toolbarColors(css, hovered);
        const ratio = contrast(color(styles.get('color') ?? '', variables), color(styles.get('background-color') ?? '', variables));
        expect(ratio, `${theme.id} ${mode ? 'dark' : 'light'} ${hovered ? 'hover' : 'idle'}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  }
});
