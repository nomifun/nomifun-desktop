// Local-only fixture pages for visual acceptance of the real desktop browser.
// Not a product route, test dashboard, or general-purpose filesystem server.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';

const fixtures = new Map([
  ['/', new URL('../fixtures/browser_workspace.html', import.meta.url)],
  ['/dialog-initial', new URL('../fixtures/browser_dialog_initial.html', import.meta.url)],
  ['/dialog-popup', new URL('../fixtures/browser_dialog_popup.html', import.meta.url)],
]);
const server = createServer(async (request, response) => {
  const file = fixtures.get(new URL(request.url ?? '/', 'http://localhost').pathname);
  if (request.method !== 'GET' || !file) {
    response.writeHead(404).end();
    return;
  }
  try {
    const body = await readFile(file);
    response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(500).end('Fixture unavailable');
  }
});
server.listen(0, '127.0.0.1', () => {
  const address = server.address();
  if (address && typeof address !== 'string') console.log(`BROWSER_UI_FIXTURE http://127.0.0.1:${address.port}/`);
});
process.on('SIGINT', () => server.close());
process.on('SIGTERM', () => server.close());
