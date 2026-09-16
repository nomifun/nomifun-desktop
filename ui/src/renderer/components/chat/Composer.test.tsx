import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { existsSync, readFileSync } from 'node:fs';
import * as focusRing from '@/renderer/hooks/chat/useInputFocusRing';
import Composer, { ComposerSendButton } from './Composer';

let ring: ReturnType<typeof spyOn<typeof focusRing, 'useInputFocusRing'>>;
beforeEach(() => {
  ring = spyOn(focusRing, 'useInputFocusRing').mockReturnValue({ activeBorderColor: '#7162ba', inactiveBorderColor: '#cccccc', activeShadow: '0 0 4px #eeeeee' });
});
afterEach(() => { cleanup(); ring.mockRestore(); });

test('home and conversation controllers use one renderer and remove the old home renderers', () => {
  const home = readFileSync(new URL('../../pages/guid/GuidPage.tsx', import.meta.url), 'utf8');
  const conversation = readFileSync(new URL('./SendBox/index.tsx', import.meta.url), 'utf8');
  for (const source of [home, conversation]) {
    expect(source.includes('<Composer\n')).toBe(true);
    expect(source.includes('<ComposerSendButton')).toBe(true);
    expect(source.includes('<Input.TextArea')).toBe(false);
    expect(source.includes('<ResponsiveComposerRow')).toBe(false);
  }
  expect(existsSync(new URL('../../pages/guid/components/GuidInputCard.tsx', import.meta.url))).toBe(false);
  expect(existsSync(new URL('../../pages/guid/components/GuidActionRow.tsx', import.meta.url))).toBe(false);
});

test('the shared editor forwards typing and paste, but IME confirmation never submits', async () => {
  const sent: string[] = [];
  let pasted = 0;
  let compositions = 0;
  function Harness() {
    const [value, setValue] = useState('草稿');
    return <Composer inputProps={{ value, onChange: setValue,
      onPaste: () => { pasted++; },
      onCompositionStartCapture: () => { compositions++; },
      onKeyDown: event => { if (event.key === 'Enter' && !event.shiftKey) sent.push(value); },
    }} actions={<ComposerSendButton disabled={!value.trim()} onClick={() => sent.push(value)} />} />;
  }
  const page = render(<Harness />);
  const input = page.getByRole('textbox');
  fireEvent.change(input, { target: { value: '保留的文字' } });
  fireEvent.paste(input);
  expect(pasted).toBe(1);
  fireEvent.compositionStart(input);
  fireEvent.keyDown(input, { key: 'Enter' });
  fireEvent.compositionEnd(input);
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(compositions).toBe(1);
  expect(sent).toEqual([]);
  await act(async () => {});
  fireEvent.keyDown(input, { key: 'Enter' });
  fireEvent.click(page.getByTestId('sendbox-send-btn'));
  expect(sent).toEqual(['保留的文字', '保留的文字']);
});

test('focus feedback and drag state share a surface while contextual tools keep their slots', () => {
  let focused = 0;
  let blurred = 0;
  let dropped = 0;
  const props = {
    inputProps: { value: '内容', onFocus: () => { focused++; }, onBlur: () => { blurred++; } },
    header: <button type='button'>场景</button>,
    sideTools: <aside aria-label='能力栏'>MCP</aside>,
    attachments: <button type='button'>参考图片</button>,
    actions: <ComposerSendButton disabled onClick={() => {}} />,
    dragHandlers: { onDrop: () => { dropped++; } },
  };
  const page = render(<Composer {...props} />);
  const surface = page.container.querySelector<HTMLElement>('[data-composer-surface]')!;
  const idle = surface.style.borderColor;
  fireEvent.focus(page.getByRole('textbox'));
  expect(focused).toBe(1);
  expect(surface.style.borderColor).not.toBe(idle);
  fireEvent.blur(page.getByRole('textbox'));
  expect(blurred).toBe(1);
  expect(surface.style.borderColor).toBe(idle);
  expect(surface.contains(page.getByRole('button', { name: '场景' }))).toBe(true);
  expect(surface.contains(page.getByRole('complementary', { name: '能力栏' }))).toBe(true);
  expect(surface.contains(page.getByRole('button', { name: '参考图片' }))).toBe(true);
  expect((page.getByTestId('sendbox-send-btn') as HTMLButtonElement).disabled).toBe(true);
  page.rerender(<Composer {...props} isFileDragging />);
  expect(surface.className).toContain('sendbox-panel--dragging');
  fireEvent.drop(surface);
  expect(dropped).toBe(1);
});
