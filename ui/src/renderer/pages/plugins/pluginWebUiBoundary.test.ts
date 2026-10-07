import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { updatePluginLibraryState } from './pluginLibraryState';

afterEach(() => mock.restore());

test('remote WebUI rejects library-state mutation without issuing a request', async () => {
  const read = spyOn(pluginPlatform.libraryState.get, 'invoke');
  const write = spyOn(pluginPlatform.libraryState.update, 'invoke');
  await expect(updatePluginLibraryState(() => undefined)).rejects.toThrow(
    'read-only in WebUI',
  );
  expect(read).not.toHaveBeenCalled();
  expect(write).not.toHaveBeenCalled();
});
