/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MattingClient } from '@renderer/services/matting/mattingClient';
import * as mattingClient from '@renderer/services/matting/mattingClient';
import type { MatteResponse } from '@renderer/services/matting/mattingProtocol';
import nomi from '@renderer/services/i18n/locales/en-US/nomi.json';
import CustomFigureWizard from './index';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { nomi } } } });

afterEach(() => { cleanup(); mock.restore(); });

test.each(['MATTE_FAILED', 'worker decoding failed'])('failed cutout (%s) retains retry, tolerance and original-image recovery controls', async message => {
  const run = spyOn(MattingClient.prototype, 'run').mockImplementation((request, receive) => {
    receive({ id: request.id, type: 'error', message });
  });
  const view = render(<I18nextProvider i18n={i18n}>
    <CustomFigureWizard open onClose={() => {}} onDone={() => {}} />
  </I18nextProvider>);
  const input = document.querySelector('input[type="file"]');
  expect(input).toBeTruthy();
  fireEvent.change(input!, { target: { files: [new File(['source'], 'source.png', { type: 'image/png' })] } });
  await waitFor(() => expect(view.getByText(message === 'MATTE_FAILED' ? nomi.customFigure.errMatteFailed : nomi.customFigure.errFailed)).toBeTruthy());
  expect(view.getByRole('button', { name: nomi.customFigure.useOriginal })).toBeTruthy();
  expect(view.getByRole('slider')).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: nomi.customFigure.retryAi }));
  expect(run.mock.calls.at(-1)?.[0].mode).toBe('auto');
  fireEvent.click(view.getByRole('button', { name: nomi.customFigure.redo }));
  expect(run.mock.calls.at(-1)?.[0].mode).toBe('heuristic');
});

test('preparation can be skipped with the original image, and late work is disposed on close', async () => {
  let receive!: (message: MatteResponse) => void;
  spyOn(MattingClient.prototype, 'run').mockImplementation((request, callback) => {
    receive = callback;
    callback({ id: request.id, type: 'progress', phase: 'download' });
  });
  const dispose = spyOn(MattingClient.prototype, 'dispose');
  const app = (open: boolean) => <I18nextProvider i18n={i18n}>
    <CustomFigureWizard open={open} onClose={() => {}} onDone={() => {}} />
  </I18nextProvider>;
  const view = render(app(true));
  fireEvent.change(document.querySelector('input[type="file"]')!, {
    target: { files: [new File(['source'], 'source.png', { type: 'image/png' })] },
  });
  await waitFor(() => expect(view.getByRole('button', { name: nomi.customFigure.useOriginal })).toBeTruthy());
  expect(view.getByRole('button', { name: nomi.customFigure.useOriginal }).hasAttribute('disabled')).toBe(false);
  expect(view.getByRole('button', { name: nomi.customFigure.retryAi }).hasAttribute('disabled')).toBe(true);
  view.rerender(app(false));
  expect(dispose).toHaveBeenCalled();
  // The wizard's monotonic request identity rejects already queued callbacks.
  receive({ id: 1, type: 'error', message: 'late worker error' });
  view.rerender(app(true));
  await waitFor(() => expect(view.queryByText(/late worker error/)).toBeNull());
});

test('using the original image starts only one decode while its encoding is pending', async () => {
  spyOn(MattingClient.prototype, 'run').mockImplementation((request, receive) => {
    receive({ id: request.id, type: 'error', message: 'MATTE_FAILED' });
  });
  let finishDecode!: (value: ImageBitmap) => void;
  const decode = spyOn(mattingClient, 'createBitmapClamped').mockImplementation(() =>
    new Promise<ImageBitmap>(resolve => { finishDecode = resolve; })
  );
  const app = (open: boolean) => <I18nextProvider i18n={i18n}>
    <CustomFigureWizard open={open} onClose={() => {}} onDone={() => {}} />
  </I18nextProvider>;
  const view = render(app(true));
  fireEvent.change(document.querySelector('input[type="file"]')!, {
    target: { files: [new File(['source'], 'source.png', { type: 'image/png' })] },
  });
  const original = view.getByRole('button', { name: nomi.customFigure.useOriginal });
  fireEvent.click(original);
  fireEvent.click(original);
  expect(decode).toHaveBeenCalledTimes(1);
  expect(original.hasAttribute('disabled')).toBe(true);
  view.rerender(app(false));
  const close = mock(() => {});
  await act(async () => { finishDecode({ width: 12, height: 12, close } as unknown as ImageBitmap); });
  expect(close).toHaveBeenCalledTimes(1);
});
