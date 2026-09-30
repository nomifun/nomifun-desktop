import '../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ConversationImagePreview } from './ConversationCreationTasks';

afterEach(cleanup);

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

test('opens the shared full-screen image preview with redundant exit controls', async () => {
  const page = render(
    <I18nextProvider i18n={testI18n}>
      <ConversationImagePreview src='/cat.png' title='猫咪' />
    </I18nextProvider>
  );
  const trigger = page.getByRole('button', { name: '预览图片：猫咪' });
  expect(page.queryByRole('dialog', { name: '查看图片' })).toBeNull();

  fireEvent.click(trigger);
  const dialog = await page.findByRole('dialog', { name: '查看图片' });
  expect(dialog.className).toContain('nomifun-modal-fullscreen');
  const images = [...page.container.querySelectorAll('img'), ...document.body.querySelectorAll('.arco-modal-wrapper img')];
  expect(images).toHaveLength(2);
  expect(images[1].getAttribute('src')).toBe('/cat.png');
  expect(page.getByRole('button', { name: '关闭图片预览' })).toBeTruthy();

  fireEvent.click(page.getByRole('button', { name: '关闭图片预览' }));
  const wrapper = dialog.closest('.arco-modal-wrapper') as HTMLElement;
  await waitFor(() => expect(wrapper.isConnected).toBe(false));

  fireEvent.click(trigger);
  const escapeDialog = await page.findByRole('dialog', { name: '查看图片' });
  const focusLock = escapeDialog.querySelector('[data-focus-lock-disabled]') as HTMLElement;
  fireEvent.keyDown(focusLock, { key: 'Escape', code: 'Escape' });
  await waitFor(() => expect(escapeDialog.isConnected).toBe(false));

  fireEvent.click(trigger);
  const maskDialog = await page.findByRole('dialog', { name: '查看图片' });
  const previewImage = document.body.querySelector('.arco-modal-wrapper img') as HTMLImageElement;
  fireEvent.click(previewImage.parentElement!);
  await waitFor(() => expect(maskDialog.isConnected).toBe(false));
});
