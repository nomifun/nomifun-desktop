import { expect, test } from 'bun:test';
import { patchSubtitleText, safeSubtitleOffset, subtitleTextMap } from './subtitleText';

test('Enter DIV/BR and empty-line placeholders use the same UTF-16 offsets as the saved plain text', () => {
  const root = document.createElement('span'); root.innerHTML = '甲🙂<div>第二行</div><div><br></div><div><br></div>';
  const map = subtitleTextMap(root);
  expect(map.text).toBe('甲🙂\n第二行\n\n');
  const second = root.querySelector('div')!.firstChild!;
  expect(map.offset(second, 1)).toBe(5);
  expect(map.locate(5)).toEqual([second, 1]);
  expect(safeSubtitleOffset(map.text, 2)).toBe(1);
  const original = root.firstChild;
  patchSubtitleText(root, map.text);
  expect(root.firstChild).toBe(original);
  expect(root.querySelector('div,br')).toBeNull();
  expect(subtitleTextMap(root).text).toBe(map.text);
});

test('incremental final text retains the original prefix node and replaces whole surrogate pairs', () => {
  const root = document.createElement('span'); root.textContent = '甲🙂乙';
  const first = root.firstChild;
  patchSubtitleText(root, '甲🙂新乙');
  expect(root.firstChild).toBe(first);
  expect(subtitleTextMap(root).text).toBe('甲🙂新乙');
  patchSubtitleText(root, '甲🨙新乙');
  expect(subtitleTextMap(root).text).toBe('甲🨙新乙');
});

test('interim span is excluded from canonical text and offsets', () => {
  const root = document.createElement('span'); root.innerHTML = '甲<span class="keyboard-subtitle__partial">暂定🙂</span>乙';
  const map = subtitleTextMap(root);
  expect(map.text).toBe('甲乙');
  expect(map.offset(root.lastChild!, 1)).toBe(2);
});
