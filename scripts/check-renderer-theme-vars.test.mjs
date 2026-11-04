import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  declaredVariables,
  findFixedWhiteOnThemeFill,
  findMissingVariables,
  referencedVariables,
} from './check-renderer-theme-vars.mjs';

const source = (path, content) => ({ path, source: content });

test('missing theme variables produce file and line diagnostics while declared and Arco variables resolve', () => {
  const sources = [
    source('renderer/theme.css', ':root { --primary: #123; }'),
    source('renderer/card.css', '.card { color: var(--primary); border: var(--arco-color);\n  background: var(--missing); }'),
  ];
  assert.deepEqual(findMissingVariables(sources, ':root { --arco-color: #456; }'), [
    { path: 'renderer/card.css', name: '--missing', line: 2 },
  ]);
});

test('local fallbacks, runtime style declarations and Uno generated variables do not produce false alarms', () => {
  const sources = [
    source('renderer/Control.tsx', `const style = { '--runtime-accent': '#123' };
element.style.setProperty('--imperative-accent', color);
const css = 'var(--runtime-accent) var(--imperative-accent) var(--optional, #fff) var(--un-text-opacity)';`),
  ];
  assert.deepEqual(findMissingVariables(sources), []);
  assert(declaredVariables(sources[0].source).has('--runtime-accent'));
});

test('nested fallbacks still validate the final no-fallback variable and comments preserve line numbers', () => {
  const sources = [source('renderer/example.css', `/* var(--commented) */
.item { color: var(--optional, var(--missing)); }
// var(--also-commented)
.other { color: var(--safe, var(--defined)); }
:root { --defined: red; }`)];
  assert.deepEqual(referencedVariables(sources[0].source).map(({ name }) => name), ['--missing', '--defined']);
  assert.deepEqual(findMissingVariables(sources), [{ path: 'renderer/example.css', name: '--missing', line: 2 }]);
});

test('fixed white text on a theme accent fill is rejected without catching media overlays', () => {
  const sources = [
    source('renderer/button.css', '.button { background: var(--primary); color: #fff; }'),
    source('renderer/danger.css', '.danger { color: var(--inverse) !important; background-color: var(--danger) !important; }'),
    source('renderer/ramp.css', '.ramp { background: rgb(var(--primary-6)); color: white; }'),
    source('renderer/Button.tsx', "const cls = 'bg-brand px-2 text-white';"),
    source('renderer/image.css', '.overlay { background: rgba(0, 0, 0, .7); color: white; }'),
    source('renderer/Image.tsx', "const cls = 'bg-[rgba(0,0,0,.7)] text-white';"),
  ];
  assert.deepEqual(findFixedWhiteOnThemeFill(sources), [
    { path: 'renderer/button.css', line: 1, reason: 'theme accent fill paired with fixed white text' },
    { path: 'renderer/danger.css', line: 1, reason: 'theme accent fill paired with fixed white text' },
    { path: 'renderer/ramp.css', line: 1, reason: 'theme accent fill paired with fixed white text' },
    { path: 'renderer/Button.tsx', line: 1, reason: 'theme accent class paired with text-white' },
  ]);
});
