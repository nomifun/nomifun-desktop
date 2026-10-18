/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { configService } from '@/common/config/configService';
import * as authContext from '@renderer/hooks/context/AuthContext';
import * as modelCache from '@renderer/services/matting/modelCache';
import ProtectedAppRuntime from './ProtectedAppRuntime';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: {} });

afterEach(() => {
  cleanup();
  mock.restore();
});

test('prepares the optional cutout model after login without delaying or failing the app', async () => {
  let status: ReturnType<typeof authContext.useAuth>['status'] = 'unauthenticated';
  spyOn(authContext, 'useAuth').mockImplementation(() => ({ status } as ReturnType<typeof authContext.useAuth>));
  spyOn(configService, 'get').mockReturnValue(undefined);
  spyOn(configService, 'set').mockResolvedValue(undefined);
  let failPreparation!: (error: Error) => void;
  const prepare = spyOn(modelCache, 'ensureMattingModel').mockImplementation(() =>
    new Promise<Blob>((_resolve, reject) => { failPreparation = reject; })
  );
  const app = () => (
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/app']}>
        <Routes>
          <Route path='/login' element={<div>Login</div>} />
          <Route element={<ProtectedAppRuntime />}>
            <Route path='/app' element={<div>Ready app</div>} />
          </Route>
        </Routes>
      </MemoryRouter>
    </I18nextProvider>
  );
  const login = render(app());
  expect(login.getByText('Login')).toBeTruthy();
  expect(prepare).not.toHaveBeenCalled();
  login.unmount();

  status = 'authenticated';
  const view = render(app());
  expect(prepare).toHaveBeenCalledTimes(1);
  // Preparation is still pending, but the protected child has already rendered.
  expect(view.getByText('Ready app')).toBeTruthy();
  await act(async () => { failPreparation(new Error('offline')); });
  expect(view.getByText('Ready app')).toBeTruthy();
});
