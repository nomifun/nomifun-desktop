import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import ImageLightbox from './ImageLightbox';

afterEach(cleanup);

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
    const page = render(<ImageLightbox src='/original.png' title='原始图片' onClose={() => closes++} onDownload={async () => { downloads++; }} />);
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
    fireEvent.click(page.getByRole('button', { name: '下载图片' }));
    await waitFor(() => expect(downloads).toBe(1));
    fireEvent.click(page.getByRole('button', { name: '关闭图片预览' }));
    expect(closes).toBe(1);
  } finally { cleanup(); globalThis.ResizeObserver = originalObserver; }
});
