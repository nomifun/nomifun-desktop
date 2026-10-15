import '../../../../test/setup-dom.ts';
import { readFileSync } from 'node:fs';
import { Modal } from '@arco-design/web-react';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState, type ReactElement } from 'react';
import { I18nextProvider } from 'react-i18next';
import ImageLightbox from './ImageLightbox';

const i18n = createInstance();
void i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { common: { retry: '重试' } } } }, initImmediate: false });
const renderPreview = (element: ReactElement) => render(<I18nextProvider i18n={i18n}>{element}</I18nextProvider>);

afterEach(cleanup);

test('keeps the image viewport sized through the focus-lock wrapper', () => {
  const styles = readFileSync(new URL('./ImageLightbox.module.css', import.meta.url), 'utf8');
  expect(styles).toMatch(/\.modal\s*>\s*:global\(\[data-focus-lock-disabled\]\)\s*\{[^}]*height:\s*100%/s);
  expect(styles).toMatch(/\.modal\s+:global\(\.arco-modal-content\)\s*\{[^}]*height:\s*100%/s);
});

test('fits the original image, zooms, resets, downloads and closes', async () => {
  const originalObserver = globalThis.ResizeObserver;
  let resize = () => {};
  globalThis.ResizeObserver = class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  let downloads = 0;
  let closes = 0;
  try {
    const page = renderPreview(<ImageLightbox src='/original.png' title='原始图片' onClose={() => closes++} onSaveAs={async () => { downloads++; }} />);
    const image = await page.findByAltText('原始图片');
    Object.defineProperties(image.parentElement!, { clientWidth: { value: 1000 }, clientHeight: { value: 700 } });
    Object.defineProperties(image, { naturalWidth: { value: 1200 }, naturalHeight: { value: 800 } });
    act(() => resize());
    fireEvent.load(image);
    expect(page.getByRole('button', { name: '适应窗口' }).textContent).toBe('81%');
    expect(image.getAttribute('src')).toBe('/original.png');
    fireEvent.click(page.getByRole('button', { name: '放大图片' }));
    expect(page.getByRole('button', { name: '适应窗口' }).textContent).toBe('101%');
    fireEvent.click(page.getByRole('button', { name: '适应窗口' }));
    expect(page.getByRole('button', { name: '适应窗口' }).textContent).toBe('81%');
    fireEvent.click(page.getByRole('button', { name: '图片另存为' }));
    await waitFor(() => expect(downloads).toBe(1));
    fireEvent.click(page.getByRole('button', { name: '关闭图片预览' }));
    expect(closes).toBe(1);
  } finally { cleanup(); globalThis.ResizeObserver = originalObserver; }
});

test('navigates the gallery with buttons and arrow keys without looping, and saves the current image', async () => {
  const saved: number[] = [];
  function Gallery() {
    const [index, setIndex] = useState(0);
    return <ImageLightbox
      src={`/result-${index}.png`}
      title='任务图片'
      navigation={{ index, total: 3, onIndexChange: setIndex }}
      onSaveAs={async () => { saved.push(index); }}
      onClose={() => {}}
    />;
  }
  const page = renderPreview(<Gallery />);
  const previous = () => page.getByRole('button', { name: '上一张图片' }) as HTMLButtonElement;
  const next = () => page.getByRole('button', { name: '下一张图片' }) as HTMLButtonElement;
  expect(previous().disabled).toBe(true);
  expect(page.getByText('1 / 3')).toBeTruthy();
  fireEvent.keyDown(next(), { key: 'ArrowLeft' });
  expect(page.getByAltText('任务图片').getAttribute('src')).toBe('/result-0.png');

  fireEvent.click(next());
  expect(page.getByAltText('任务图片').getAttribute('src')).toBe('/result-1.png');
  expect(page.getByText('2 / 3')).toBeTruthy();
  expect(previous().disabled).toBe(false);
  fireEvent.keyDown(next(), { key: 'ArrowRight', metaKey: true });
  expect(page.getByText('2 / 3')).toBeTruthy();
  fireEvent.keyDown(next(), { key: 'ArrowRight' });
  expect(page.getByText('3 / 3')).toBeTruthy();
  expect(next().disabled).toBe(true);
  fireEvent.keyDown(previous(), { key: 'ArrowRight' });
  expect(page.getByAltText('任务图片').getAttribute('src')).toBe('/result-2.png');

  const image = page.getByAltText('任务图片');
  Object.defineProperties(image, { naturalWidth: { value: 1200 }, naturalHeight: { value: 800 } });
  fireEvent.load(image);
  fireEvent.click(page.getByRole('button', { name: '图片另存为' }));
  await waitFor(() => expect(saved).toEqual([2]));

  fireEvent.keyDown(previous(), { key: 'ArrowLeft' });
  expect(page.getByText('2 / 3')).toBeTruthy();
  expect(page.getByRole('button', { name: '适应窗口' }).textContent).toBe('—');
  fireEvent.click(previous());
  expect(page.getByText('1 / 3')).toBeTruthy();
});

test('does not add gallery controls or consume arrow keys for a single image', () => {
  const changes: number[] = [];
  const page = renderPreview(<ImageLightbox src='/single.png' title='单张图片' navigation={{ index: 0, total: 1, onIndexChange: (index) => changes.push(index) }} onClose={() => {}} />);
  expect(page.queryByRole('button', { name: '上一张图片' })).toBeNull();
  expect(page.queryByRole('button', { name: '下一张图片' })).toBeNull();
  expect(page.queryByText('1 / 1')).toBeNull();
  const event = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
  page.getByRole('button', { name: '关闭图片预览' }).dispatchEvent(event);
  expect(event.defaultPrevented).toBe(false);
  expect(changes).toEqual([]);
});

test('does not consume arrow keys from an external input or a dialog above the image preview', () => {
  const changes: number[] = [];
  const page = renderPreview(<>
    <input aria-label='外部输入' />
    <ImageLightbox src='/current.png' title='当前图片' navigation={{ index: 1, total: 3, onIndexChange: (index) => changes.push(index) }} onClose={() => {}} />
    <Modal visible title='Exception details' footer={null} focusLock>
      <input aria-label='异常说明' />
    </Modal>
  </>);
  expect(page.getByAltText('当前图片')).toBeTruthy();
  for (const label of ['外部输入', '异常说明']) {
    const input = page.getByRole('textbox', { name: label });
    for (const key of ['ArrowLeft', 'ArrowRight']) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      fireEvent(input, event);
      expect(event.defaultPrevented).toBe(false);
    }
  }
  expect(changes).toEqual([]);
  expect(page.getByText('2 / 3')).toBeTruthy();
});

test('keeps gallery navigation available when an image is unavailable', () => {
  let retries = 0;
  function Gallery() {
    const [index, setIndex] = useState(0);
    return <ImageLightbox
      src={index ? '/available.png' : null}
      title='任务结果'
      error={index ? null : '图片已不可用'}
      navigation={{ index, total: 2, onIndexChange: setIndex }}
      onRetry={() => retries++}
      onSaveAs={async () => {}}
      onClose={() => {}}
    />;
  }
  const page = renderPreview(<Gallery />);
  expect(page.getByRole('alert').textContent).toContain('图片已不可用');
  expect((page.getByRole('button', { name: '图片另存为' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(page.getByRole('button', { name: '重试' }));
  expect(retries).toBe(1);
  fireEvent.click(page.getByRole('button', { name: '下一张图片' }));
  expect(page.getByAltText('任务结果').getAttribute('src')).toBe('/available.png');
  expect(page.queryByRole('alert')).toBeNull();
  expect(page.getByText('2 / 2')).toBeTruthy();
});
