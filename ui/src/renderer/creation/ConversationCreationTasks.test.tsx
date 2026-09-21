import '../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { ConversationImagePreview } from './ConversationCreationTasks';

afterEach(cleanup);

test('opens a bounded accessible preview with redundant exit controls', async () => {
  const page = render(<ConversationImagePreview src='/cat.png' title='猫咪' />);
  const trigger = page.getByRole('button', { name: '预览图片：猫咪' });
  expect(page.queryByRole('dialog', { name: '图片预览' })).toBeNull();

  fireEvent.click(trigger);
  const dialog = await page.findByRole('dialog', { name: '图片预览' });
  expect(dialog.className).not.toContain('nomifun-modal-fullscreen');
  const images = page.getAllByRole('img', { name: '猫咪' });
  expect(images).toHaveLength(2);
  expect(images[1].getAttribute('src')).toBe('/cat.png');
  expect(page.getByRole('button', { name: '关闭图片预览' })).toBeTruthy();

  fireEvent.click(page.getByRole('button', { name: '关闭图片预览' }));
  const wrapper = dialog.closest('.arco-modal-wrapper') as HTMLElement;
  await waitFor(() => expect(wrapper.isConnected).toBe(false));

  fireEvent.click(trigger);
  const escapeDialog = await page.findByRole('dialog', { name: '图片预览' });
  const focusLock = escapeDialog.querySelector('[data-focus-lock-disabled]') as HTMLElement;
  fireEvent.keyDown(focusLock, { key: 'Escape', code: 'Escape' });
  await waitFor(() => expect(escapeDialog.isConnected).toBe(false));

  fireEvent.click(trigger);
  const maskDialog = await page.findByRole('dialog', { name: '图片预览' });
  const maskWrapper = maskDialog.closest('.arco-modal-wrapper') as HTMLElement;
  fireEvent.mouseDown(maskWrapper);
  fireEvent.click(maskWrapper);
  await waitFor(() => expect(maskDialog.isConnected).toBe(false));
});
