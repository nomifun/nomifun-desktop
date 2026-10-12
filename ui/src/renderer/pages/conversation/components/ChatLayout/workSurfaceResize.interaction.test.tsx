import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import { initializeAgentBrowserStorageGeneration, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import * as Preview from '../../Preview';
import * as Title from '../ChatTitleEditor';
import ChatLayout from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000731');
const splitStorageKey = 'chat-preview-split-ratio';
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth');
const originalInnerWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth');
let paneWidth = 1200;

beforeEach(() => {
  paneWidth = 1200;
  localStorage.clear();
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000732');
  initializeAgentBrowserStorageGeneration(1);
  // Happy DOM does not perform CSS layout. Supply the measured desktop pane
  // width while retaining the real container observer and resize hook.
  HTMLElement.prototype.getBoundingClientRect = () => ({
    x: 0, y: 0, width: paneWidth, height: 600,
    left: 0, top: 0, right: paneWidth, bottom: 600, toJSON: () => ({}),
  });
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
    configurable: true, get: () => paneWidth - (document.querySelector('.workspace-tool-rail') ? 32 : 0),
  });
  Object.defineProperty(window, 'innerWidth', { configurable: true, get: () => Math.max(880, paneWidth) });
  spyOn(ipcBridge.preview.open, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  // Document rendering and the title editor are independent of the shared
  // layout. Keep the real browser panel (WebUI has no native attachment),
  // tool rail, provider, sizing, pointer handlers and persistence.
  spyOn(Preview, 'PreviewPanel').mockImplementation(() => <div>Attachment content</div>);
  spyOn(Title, 'default').mockImplementation(() => <span>Resize fixture</span>);
});

afterEach(() => {
  cleanup();
  mock.restore();
  localStorage.clear();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  if (originalOffsetWidth) Object.defineProperty(HTMLElement.prototype, 'offsetWidth', originalOffsetWidth);
  else Reflect.deleteProperty(HTMLElement.prototype, 'offsetWidth');
  if (originalInnerWidth) Object.defineProperty(window, 'innerWidth', originalInnerWidth);
  else Reflect.deleteProperty(window, 'innerWidth');
});

function Composer() {
  const preview = Preview.usePreviewContext();
  return <div>
    <p>Chat content</p>
    <button type='button' onClick={() => preview.openPreview('Attachment', 'markdown', { file_name: 'fixture.md' })}>Open attachment</button>
  </div>;
}

function mountLayout(workspaceEnabled = true, embedded = false) {
  return render(<I18nextProvider i18n={i18n}>
    <ChatLayout conversation_id={conversationId} title='Resize fixture' hideAdvancedControls workspaceEnabled={workspaceEnabled} embedded={embedded} sider={null}>
      <Composer />
    </ChatLayout>
  </I18nextProvider>);
}

function panels(container: HTMLElement, kind: 'attachment' | 'browser') {
  const surface = container.querySelector<HTMLElement>(kind === 'attachment' ? '.preview-panel' : '.browser-capability-surface');
  expect(surface).not.toBeNull();
  const chat = surface!.previousElementSibling as HTMLElement;
  expect(chat.textContent).toContain('Chat content');
  const handle = surface!.querySelector<HTMLElement>('.cursor-col-resize');
  expect(handle).not.toBeNull();
  return { surface: surface!, chat, handle: handle! };
}

async function drag(handle: HTMLElement, from: number, to: number) {
  await act(async () => {
    fireEvent.pointerDown(handle, { pointerId: 1, pointerType: 'mouse', button: 0, buttons: 1, clientX: from });
    fireEvent.pointerMove(window, { pointerId: 1, pointerType: 'mouse', buttons: 1, clientX: to });
    fireEvent.pointerUp(window, { pointerId: 1, pointerType: 'mouse', button: 0, buttons: 0, clientX: to });
  });
  expect(document.body.style.userSelect).toBe('');
  expect(document.body.style.cursor).toBe('');
  return Number(localStorage.getItem(splitStorageKey));
}

test('attachment and browser share pointer resizing and retain the saved width across switches and remounts', async () => {
  let view = mountLayout();
  fireEvent.click(view.getByRole('button', { name: 'Open attachment' }));
  const attachment = panels(view.container, 'attachment');
  const attachmentRatio = await drag(attachment.handle, 800, 600);
  expect(attachmentRatio).toBeCloseTo(60 - 200 / (paneWidth - 32) * 100);
  expect(parseFloat(attachment.chat.style.flexBasis)).toBeCloseTo(attachmentRatio);

  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  const browser = panels(view.container, 'browser');
  expect(browser.surface.style.minWidth).toBe(attachment.surface.style.minWidth);
  expect(browser.chat.style.flexBasis).toBe(attachment.chat.style.flexBasis);
  expect(browser.handle.style.cssText).toBe(attachment.handle.style.cssText);
  const browserRatio = await drag(browser.handle, 600, 800);
  expect(browserRatio).toBeCloseTo(attachmentRatio + 200 / (paneWidth - 32) * 100);
  expect(parseFloat(browser.chat.style.flexBasis)).toBeCloseTo(browserRatio);

  fireEvent.click(view.getByRole('button', { name: 'Open attachment' }));
  expect(parseFloat(panels(view.container, 'attachment').chat.style.flexBasis)).toBeCloseTo(browserRatio);
  view.unmount();
  view = mountLayout();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  expect(parseFloat(panels(view.container, 'browser').chat.style.flexBasis)).toBeCloseTo(browserRatio);
});

for (const workspaceEnabled of [true, false]) {
  test(`the minimum desktop pane keeps both surfaces resizable within pixel limits ${workspaceEnabled ? 'with' : 'without'} a workspace`, async () => {
    paneWidth = 880;
    const view = mountLayout(workspaceEnabled);
    fireEvent.click(view.getByRole('button', { name: 'Open attachment' }));
    const attachment = panels(view.container, 'attachment');
    expect(attachment.chat.style.display).not.toBe('none');
    const attachmentMin = await drag(attachment.handle, 700, -1000);
    const attachmentMax = await drag(attachment.handle, 0, 2000);
    expect(attachmentMin).toBeCloseTo(360 / (paneWidth - 32) * 100);
    expect(attachmentMax).toBeCloseTo(100 - 340 / (paneWidth - 32) * 100);

    fireEvent.click(view.getByRole('button', { name: 'Browser' }));
    const browser = panels(view.container, 'browser');
    expect(browser.chat.style.display).not.toBe('none');
    expect(browser.surface.style.minWidth).toBe(attachment.surface.style.minWidth);
    expect(parseFloat(browser.chat.style.flexBasis)).toBeCloseTo(attachmentMax);
    expect(await drag(browser.handle, 700, -1000)).toBeCloseTo(attachmentMin);
    expect(await drag(browser.handle, 0, 2000)).toBeCloseTo(attachmentMax);
  });
}

test('a narrow embedded desktop pane preserves its smaller chat minimum and fits both drag limits', async () => {
  paneWidth = 500;
  expect(window.innerWidth).toBe(880);
  const view = mountLayout(false, true);
  expect(view.container.querySelector('.workspace-tool-rail')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Open attachment' }));
  const attachment = panels(view.container, 'attachment');
  expect(attachment.chat.style.minWidth).toBe('0px');

  const maxRatio = await drag(attachment.handle, 0, 2000);
  expect(maxRatio).toBeCloseTo(100 - 340 / paneWidth * 100);
  const minRatio = await drag(attachment.handle, 700, -1000);
  expect(minRatio).toBeCloseTo(25);
  for (const ratio of [minRatio, maxRatio]) {
    const chatWidth = paneWidth * ratio / 100;
    expect(chatWidth).toBeLessThan(360);
    expect(chatWidth + parseFloat(attachment.surface.style.minWidth)).toBeLessThanOrEqual(paneWidth);
  }
});
