import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useNavigate, useParams } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import { initializeAgentBrowserStorageGeneration, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { LayoutContext, useLayoutContext, type ReserveWorkSurfaceSpace, type WorkSurfaceSpaceRequest } from '@/renderer/hooks/context/LayoutContext';
import { sessionSiderChannel } from '@/renderer/utils/workspace/sessionSiderEvents';
import * as Sessions from '../../SessionList';
import * as Header from './SessionCreateBar';
import * as Title from '../ChatTitleEditor';
import * as Preview from '../../Preview';
import type { PreviewContextValue } from '../../Preview/context/PreviewContext';
import ChatLayout from '../ChatLayout';
import ConversationShell from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
const firstSession = '0190f5fe-7c00-7a00-8000-000000000751';
const secondSession = '0190f5fe-7c00-7a00-8000-000000000752';
const collapseKey = 'nomifun:session-sider-collapsed';
const widthKey = 'nomifun:session-sider-width';
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalInnerWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth');
let shellWidth = 1100;
const primaryReservations = new Set<WorkSurfaceSpaceRequest>();
let reserveFromChild: ReserveWorkSurfaceSpace | undefined;
let previewContext: PreviewContextValue | undefined;
const setSiderCollapsed = mock((_value: boolean) => {});
const reserveSiderSpace = (request: WorkSurfaceSpaceRequest) => {
  primaryReservations.add(request);
  return () => { primaryReservations.delete(request); };
};

beforeEach(() => {
  shellWidth = 1100;
  localStorage.clear();
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000753');
  initializeAgentBrowserStorageGeneration(1);
  localStorage.setItem(collapseKey, 'expanded');
  localStorage.setItem(widthKey, '300');
  primaryReservations.clear();
  reserveFromChild = undefined;
  previewContext = undefined;
  setSiderCollapsed.mockClear();
  // Happy DOM has no CSS geometry. Measure the complete shell separately from
  // its child pane, retaining the real observer, route and sidebar ownership.
  HTMLElement.prototype.getBoundingClientRect = function () {
    const childPane = this.classList.contains('w-full') && this.classList.contains('overflow-hidden');
    const width = shellWidth - (childPane && document.querySelector('.content-sider') ? 300 : 0);
    return { x: 0, y: 0, width, height: 600, left: 0, top: 0, right: width, bottom: 600, toJSON: () => ({}) };
  };
  Object.defineProperty(window, 'innerWidth', { configurable: true, get: () => Math.max(880, shellWidth) });
  spyOn(Sessions, 'default').mockImplementation(() => <p>Session list contents</p>);
  spyOn(Header, 'default').mockImplementation(({ onCollapse }) => <button onClick={onCollapse}>Collapse session list</button>);
  spyOn(Title, 'default').mockImplementation(() => <span>Work surface fixture</span>);
  // Keep the real provider and edited tab state; document rendering does not
  // participate in sidebar ownership.
  spyOn(Preview, 'PreviewPanel').mockImplementation(() => <p>Preview contents</p>);
  spyOn(ipcBridge.preview.open, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
});

afterEach(() => {
  cleanup();
  expect(primaryReservations.size).toBe(0);
  mock.restore();
  localStorage.clear();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  if (originalInnerWidth) Object.defineProperty(window, 'innerWidth', originalInnerWidth);
  else Reflect.deleteProperty(window, 'innerWidth');
});

function SessionFixture() {
  const { id } = useParams();
  const navigate = useNavigate();
  reserveFromChild = useLayoutContext()?.reserveWorkSurfaceSpace;
  if (!id) return <p>Welcome route</p>;
  return <ChatLayout conversation_id={parseConversationId(id)} title='Work surface fixture' hideAdvancedControls workspaceEnabled={false} sider={null}>
    <p>Chat content</p>
    <PreviewControls />
    <button onClick={() => navigate(`/conversation/${secondSession}`)}>Switch session</button>
    <button onClick={() => navigate('/guid')}>Leave session</button>
  </ChatLayout>;
}

function PreviewControls() {
  previewContext = Preview.usePreviewContext();
  return <>
    <button onClick={() => previewContext!.openPreview('Original text', 'code', { file_name: 'draft.txt', editable: true })}>Open editable preview</button>
    <button onClick={() => previewContext!.updateContent('Unsaved draft')}>Edit preview</button>
  </>;
}

function mountShell() {
  return render(<I18nextProvider i18n={i18n}>
    <LayoutContext.Provider value={{ siderCollapsed: false, setSiderCollapsed, reserveSiderSpace }}>
      <MemoryRouter initialEntries={[`/conversation/${firstSession}`]}>
        <Routes><Route element={<ConversationShell />}>
          <Route path='/conversation/:id' element={<SessionFixture />} />
          <Route path='/guid' element={<SessionFixture />} />
        </Route></Routes>
      </MemoryRouter>
    </LayoutContext.Provider>
  </I18nextProvider>);
}

test('the complete shell temporarily yields its list and restores its saved width without remounting the browser', async () => {
  const view = mountShell();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  const browser = view.getByRole('region', { name: 'Browser' });
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(primaryReservations.size).toBe(1);

  shellWidth = 880;
  await act(async () => { fireEvent(window, new Event('resize')); });
  expect(view.queryByText('Session list contents')).toBeNull();
  expect(sessionSiderChannel.getSnapshot().collapsed).toBe(true);
  expect(localStorage.getItem(collapseKey)).toBe('expanded');
  expect(localStorage.getItem(widthKey)).toBe('300');
  expect(view.getByRole('region', { name: 'Browser' })).toBe(browser);

  shellWidth = 1100;
  await act(async () => { fireEvent(window, new Event('resize')); });
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(sessionSiderChannel.getSnapshot().collapsed).toBe(false);
  expect(view.container.querySelector<HTMLElement>('.content-sider')!.style.width).toBe('300px');
  expect(view.getByRole('region', { name: 'Browser' })).toBe(browser);
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  expect(primaryReservations.size).toBe(0);
});

test('an explicit list expansion hides the work panel and expands the existing list', async () => {
  shellWidth = 880;
  const view = mountShell();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  expect(view.queryByText('Session list contents')).toBeNull();
  await act(async () => { sessionSiderChannel.dispatchToggle(); });
  expect(view.queryByRole('region', { name: 'Browser' })).toBeNull();
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(primaryReservations.size).toBe(0);
  expect(localStorage.getItem(widthKey)).toBe('300');
});

test('expanding the session list preserves an open Preview and its unsaved edit at the minimum desktop width', async () => {
  shellWidth = 880;
  const view = mountShell();
  fireEvent.click(view.getByRole('button', { name: 'Open editable preview' }));
  fireEvent.click(view.getByRole('button', { name: 'Edit preview' }));
  const editedTab = previewContext!.activeTab;
  expect(editedTab?.content).toBe('Unsaved draft');
  expect(editedTab?.isDirty).toBe(true);
  expect(primaryReservations.size).toBe(0);
  expect(setSiderCollapsed).toHaveBeenCalledWith(true);

  // The session list retains its existing toggle behavior. Reopening it must
  // not route through Preview's destructive close operation.
  await act(async () => { sessionSiderChannel.dispatchToggle(); });
  expect(view.queryByText('Session list contents')).toBeNull();
  await act(async () => { sessionSiderChannel.dispatchToggle(); });
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(previewContext!.isOpen).toBe(true);
  expect(previewContext!.tabs).toHaveLength(1);
  expect(previewContext!.activeTab?.id).toBe(editedTab!.id);
  expect(previewContext!.activeTab?.content).toBe('Unsaved draft');
  expect(previewContext!.activeTab?.isDirty).toBe(true);
});

test('switching from Browser to Preview restores Preview sidebar ownership and releases the browser reservation', async () => {
  shellWidth = 880;
  const view = mountShell();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  expect(primaryReservations.size).toBe(1);
  expect(setSiderCollapsed).not.toHaveBeenCalled();

  fireEvent.click(view.getByRole('button', { name: 'Open editable preview' }));
  expect(view.queryByRole('region', { name: 'Browser' })).toBeNull();
  expect(primaryReservations.size).toBe(0);
  expect(setSiderCollapsed).toHaveBeenLastCalledWith(true);
  await act(async () => { previewContext!.closePreview(); });
  expect(setSiderCollapsed).toHaveBeenLastCalledWith(false);
});

test('session replacement and route departure release only the exact mounted space request', async () => {
  shellWidth = 880;
  const view = mountShell();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  const firstRequest = [...primaryReservations][0];
  await act(async () => { fireEvent.click(view.getByRole('button', { name: 'Switch session' })); });
  // ChatLayout's Session-scoped PreviewProvider remounts its inner surface.
  expect(primaryReservations.size).toBe(0);
  expect(view.queryByRole('region', { name: 'Browser' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Browser' }));
  expect(primaryReservations.size).toBe(1);
  expect(primaryReservations.has(firstRequest)).toBe(false);
  expect(view.getByRole('region', { name: 'Browser' }).id).toContain(secondSession);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: 'Leave session' })); });
  expect(primaryReservations.size).toBe(0);
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(localStorage.getItem(collapseKey)).toBe('expanded');
});

test('a late release from an old child cannot clear its successor reservation', async () => {
  shellWidth = 880;
  const view = mountShell();
  const first = { minimumWidth: 732, close: () => {} };
  const successor = { minimumWidth: 732, close: () => {} };
  let releaseFirst!: () => void;
  let releaseSuccessor!: () => void;
  await act(async () => { releaseFirst = reserveFromChild!(first); });
  await act(async () => { releaseSuccessor = reserveFromChild!(successor); });
  expect(primaryReservations.size).toBe(1);
  expect(primaryReservations.has(successor)).toBe(true);
  await act(async () => { releaseFirst(); });
  expect(primaryReservations.size).toBe(1);
  expect(primaryReservations.has(successor)).toBe(true);
  expect(view.queryByText('Session list contents')).toBeNull();
  await act(async () => { releaseSuccessor(); });
  expect(primaryReservations.size).toBe(0);
  expect(view.getByText('Session list contents')).toBeDefined();
  expect(localStorage.getItem(collapseKey)).toBe('expanded');
});
