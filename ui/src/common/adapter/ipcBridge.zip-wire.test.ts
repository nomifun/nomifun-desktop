import { afterEach, expect, test } from 'bun:test';
import { fs, type IZipFile } from './ipcBridge';

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

test('ZIP request preserves disk sources and batch workspace scopes on the wire', async () => {
  let requestUrl = '';
  let requestBody: unknown;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    requestUrl = String(input);
    requestBody = JSON.parse(String(init?.body));
    return new Response(JSON.stringify({ success: true, data: true }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;

  const files: IZipFile[] = [
    { name: 'hello/conversation.json', content: '{"messages":[]}' },
    { name: 'hello/workspace/nested/image.png', file_path: 'D:\\projects\\hello\\nested\\image.png' },
    { name: 'other/workspace/readme.md', file_path: 'E:\\projects\\other\\readme.md' },
  ];
  const request = {
    path: 'D:\\workplace\\data\\dev-temp\\hello.zip',
    request_id: 'export-test',
    workspaces: ['D:\\projects\\hello', 'E:\\projects\\other'],
    files,
  };

  expect(await fs.createZip.invoke(request)).toBe(true);
  expect(requestUrl).toEndWith('/api/fs/zip');
  expect(requestBody).toEqual(request);
});
