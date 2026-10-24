/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';
import { Message } from '@arco-design/web-react';
import { cleanup, fireEvent, render, waitFor, within, type RenderResult } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';

import * as theme from '@/renderer/hooks/context/ThemeContext';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import common from '../../../../services/i18n/locales/en-US/common.json';
import creativeStudio from '../../../../services/i18n/locales/en-US/creativeStudio.json';
import type { CreativeAsset } from '../../assets';
import CreativeResourceDialog from '../../components/CreativeResourceDialog';
import type { CreativeTemplateRunAggregateV1 } from '../domain';
import { IDS, createTemplateRunFixture } from '../domain/testFixtures';
import type { ReviewCreativeTemplateDraft } from '../runtime';
import TemplateRunCenter, { type CreativeTemplateRunCenterPort } from './TemplateRunCenter';

const testI18n = createInstance();
await testI18n.init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: { common, creativeStudio } } },
  interpolation: { escapeValue: false },
});

const restores: Array<() => void> = [];
beforeEach(() => {
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  restores.push(() => themeSpy.mockRestore(), () => success.mockRestore());
});
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

function createPort(
  runs: readonly CreativeTemplateRunAggregateV1[] = [],
  overrides: Partial<CreativeTemplateRunCenterPort> = {}
): CreativeTemplateRunCenterPort {
  return {
    snapshot: { loading: false, loadError: null, runs, activities: {} },
    assetUrl: (assetId) => `/api/creative-studio/files/${assetId}`,
    resume: mock(async (_runId: string) => undefined),
    cancel: mock(async (_runId: string) => undefined),
    review: mock(async (_runId: string, _drafts: readonly ReviewCreativeTemplateDraft[]) => undefined),
    retry: mock(async (_run: CreativeTemplateRunAggregateV1) => undefined),
    ...overrides,
  };
}

function runFixture(status: CreativeTemplateRunAggregateV1['record']['status'], id = IDS.request as string, series = false) {
  const run = createTemplateRunFixture(series);
  run.request.id = id;
  run.request.idempotencyKey = id;
  run.record.requestId = id;
  run.record.status = status;
  run.templateSnapshot.metadata.name = `${status} template`;
  return run;
}

function assetFixture(id: string, deleted = false): CreativeAsset {
  return {
    id, kind: 'image', title: `Result ${id}`, collection: null, tags: [],
    mimeType: 'image/png', width: 720, height: 1280, bytes: 1024, inLibrary: !deleted,
    textContent: null, origin: null, originalUrl: deleted ? '' : `/original/${id}`, thumbnailUrl: null,
    createdAt: 1, updatedAt: 2, ...(deleted ? { deletedAt: 2 } : {}),
  };
}

const center = (port: CreativeTemplateRunCenterPort) => <I18nextProvider i18n={testI18n}><TemplateRunCenter port={port} /></I18nextProvider>;
const renderCenter = (port: CreativeTemplateRunCenterPort) => render(center(port));
const openList = async (view: RenderResult) => {
  fireEvent.click(view.getByRole('button', { name: 'Template runs' }));
  return view.findByRole('dialog', { name: 'Template runs' });
};
const rowFor = (view: RenderResult, run: CreativeTemplateRunAggregateV1) => {
  const row = view.baseElement.querySelector<HTMLElement>(`[data-run-id="${run.request.id}"]`);
  expect(row).not.toBeNull();
  return within(row!);
};

describe('Template Run Center', () => {
  test('opens from the shared resource task entry on its first mount', async () => {
    const port = createPort([runFixture('running')]);
    const view = render(<I18nextProvider i18n={testI18n}>
      <TemplateRunCenter port={port} triggerLabel='Template tasks' initialOpen />
    </I18nextProvider>);
    const trigger = view.getByRole('button', { name: 'Template tasks' });
    expect(trigger.textContent).toBe('Template tasks1');
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    const panel = await view.findByRole('dialog', { name: 'Template runs' });
    expect(within(panel).getByText('running template')).toBeTruthy();
    fireEvent.keyDown(panel, { key: 'Escape' });
    await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
    expect(port.resume).not.toHaveBeenCalled();
    expect(port.retry).not.toHaveBeenCalled();
  });

  test('keeps the entry visible and history closed, counts review and paused work, and returns focus when dismissed', async () => {
    const running = runFixture('running');
    const reviewing = runFixture('awaiting-review', IDS.idempotency, true);
    const paused = runFixture('queued', IDS.history);
    const completed = runFixture('succeeded', IDS.result2);
    const port = createPort([running, reviewing, paused, completed]);
    port.snapshot.activities = { [paused.request.id]: { state: 'paused', taskStatuses: {}, error: null } };
    const view = renderCenter(port);
    const trigger = view.getByRole('button', { name: 'Template runs' });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(trigger.getAttribute('title')).toContain('3 unfinished');
    expect(view.queryByRole('dialog')).toBeNull();
    expect(view.queryByText('running template')).toBeNull();

    const panel = await openList(view);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(within(panel).getAllByRole('article')).toHaveLength(4);
    expect(within(panel).getByText('Image series')).toBeTruthy();
    expect(within(panel).getByText('Waiting to resume')).toBeTruthy();
    expect(within(panel).getByText('4 runs')).toBeTruthy();
    fireEvent.keyDown(panel, { key: 'Escape' });
    await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
    expect(document.activeElement === trigger).toBe(true);
    await openList(view);
    fireEvent.click(trigger);
    await waitFor(() => expect(trigger.getAttribute('aria-expanded')).toBe('false'));
  });

  test('shows an empty state, loading state, and load failure only after opening the list', async () => {
    const port = createPort();
    const view = renderCenter(port);
    expect(view.queryByText('No template runs yet')).toBeNull();
    await openList(view);
    expect(view.getByText('No template runs yet')).toBeTruthy();
    expect(view.getByText('Run a template to see it here')).toBeTruthy();
    view.rerender(center({ ...port, snapshot: { ...port.snapshot, loading: true } }));
    expect(view.getByText('Loading template runs…')).toBeTruthy();
    expect(view.queryByText('No template runs yet')).toBeNull();
    view.rerender(center({ ...port, snapshot: { ...port.snapshot, loadError: 'database offline' } }));
    expect(view.getByRole('alert').textContent).toBe('Unable to load template runs: database offline');
    expect(view.queryByText('No template runs yet')).toBeNull();
    expect(view.queryByText('Loading template runs…')).toBeNull();
  });

  test('keeps the list inside the resource dialog focus lock and handles Escape without closing its parent', async () => {
    const closeResource = mock(() => {});
    const view = render(<I18nextProvider i18n={testI18n}>
      <CreativeResourceDialog kind='templates' title='Template library' onClose={closeResource}>
        <TemplateRunCenter port={createPort([runFixture('running')])} />
      </CreativeResourceDialog>
    </I18nextProvider>);
    const resourceDialog = view.getByRole('dialog', { name: 'Template library' });
    const focusLock = resourceDialog.querySelector('[data-focus-lock-disabled]');
    const trigger = view.getByRole('button', { name: 'Template runs' });
    const panel = await openList(view);
    expect(focusLock?.contains(panel)).toBe(true);
    expect(panel.closest('[data-focus-lock-disabled]') === focusLock).toBe(true);

    fireEvent.keyDown(trigger, { key: 'Escape' });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(closeResource).not.toHaveBeenCalled();
    expect(view.getByRole('dialog', { name: 'Template library' }) === resourceDialog).toBe(true);
    const reopened = await openList(view);
    fireEvent.keyDown(within(reopened).getByRole('button', { name: 'More task actions' }), { key: 'Escape' });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement === trigger).toBe(true);
    expect(closeResource).not.toHaveBeenCalled();
    expect(view.getByRole('dialog', { name: 'Template library' }) === resourceDialog).toBe(true);
  });

  test('uses only recorded task IDs for active progress and removes progress when the run completes', async () => {
    const run = runFixture('running');
    run.record.taskIds = [IDS.task, IDS.task2, IDS.task3];
    const port = createPort([run]);
    port.snapshot.activities = { [run.request.id]: { state: 'executing', taskStatuses: {
      [IDS.task]: 'running', [IDS.task2]: 'queued', [IDS.task3]: 'queued', unrelated: 'succeeded',
    }, error: null } };
    const view = renderCenter(port);
    await openList(view);
    const progress = view.getByRole('progressbar', { name: 'Tasks 0/3' }) as HTMLProgressElement;
    expect(progress.value).toBe(0);
    expect(progress.max).toBe(3);
    const settled = { state: 'executing' as const, taskStatuses: {
      [IDS.task]: 'succeeded' as const, [IDS.task2]: 'failed' as const, [IDS.task3]: 'canceled' as const,
    }, error: null };
    view.rerender(center({ ...port, snapshot: { ...port.snapshot, activities: { [run.request.id]: settled } } }));
    expect((view.getByRole('progressbar', { name: 'Tasks 3/3' }) as HTMLProgressElement).value).toBe(3);
    const succeeded = { ...run, record: { ...run.record, status: 'succeeded' as const } };
    view.rerender(center({ ...port, snapshot: { ...port.snapshot, runs: [succeeded], activities: {} } }));
    expect(view.getByText('Completed')).toBeTruthy();
    expect(view.queryByRole('progressbar')).toBeNull();
    expect(view.queryByText('Tasks 3/3')).toBeNull();
  });

  test('keeps unrecorded progress indeterminate and paused or review work free of progress bars', async () => {
    const unknown = runFixture('requested');
    const paused = runFixture('running', IDS.idempotency);
    paused.record.taskIds = [IDS.task];
    const reviewing = runFixture('awaiting-review', IDS.result2, true);
    reviewing.record.taskIds = [IDS.task2];
    const port = createPort([unknown, paused, reviewing]);
    port.snapshot.activities = {
      [unknown.request.id]: { state: 'executing', taskStatuses: { [IDS.task]: 'succeeded' }, error: null },
      [paused.request.id]: { state: 'paused', taskStatuses: { [IDS.task]: 'running' }, error: null },
    };
    const view = renderCenter(port);
    await openList(view);
    expect(rowFor(view, unknown).getByText('Tasks 0/-')).toBeTruthy();
    expect(view.queryByRole('progressbar')).toBeNull();
    expect(rowFor(view, paused).getByRole('button', { name: 'Resume' })).toBeTruthy();
    expect(rowFor(view, reviewing).getByRole('button', { name: 'Review prompts' })).toBeTruthy();
  });

  test('shows the shared error summary with folded, sanitized diagnostics and copies both original error sources', async () => {
    const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
    restores.push(() => copy.mockRestore());
    const run = runFixture('failed');
    run.record.failure = { code: 'task-failed', message: 'GPU queue exhausted; Authorization: Bearer durable-secret' };
    const port = createPort([run]);
    port.snapshot.activities = { [run.request.id]: { state: 'executing', taskStatuses: {}, error: 'network disconnected at https://api.example.test/v1?token=runtime-secret' } };
    const view = renderCenter(port);
    const panel = await openList(view);
    expect(within(panel).queryByText(/GPU queue exhausted|network disconnected/)).toBeNull();
    expect(within(panel).queryByRole('alert')).toBeNull();
    const failure = rowFor(view, run).getByRole('button', { name: 'Failed · View error details' });
    fireEvent.click(failure);
    const dialog = await view.findByRole('dialog', { name: 'Template task error' });
    expect(dialog.className).toContain('nomifun-modal');
    expect(dialog.getAttribute('aria-describedby')).toBeTruthy();
    const summary = within(dialog).getByText(creativeStudio.templates.runCenter.errorSummary);
    expect(summary.closest('details')).toBeNull();
    expect(summary.textContent).not.toContain('GPU queue exhausted');
    const details = dialog.querySelector('details')!;
    expect(details.open).toBe(false);
    expect(details.querySelector('summary')?.textContent).toBe(common.errorFeedback.technicalDetails);
    const technical = details.querySelector('pre')?.textContent ?? '';
    for (const value of ['task-failed', 'GPU queue exhausted', 'network disconnected', 'https://api.example.test/v1']) {
      expect(technical).toContain(value);
    }
    expect(technical.match(/GPU queue exhausted/g)).toHaveLength(1);
    expect(technical.match(/network disconnected/g)).toHaveLength(1);
    for (const secret of ['durable-secret', 'runtime-secret']) expect(dialog.textContent).not.toContain(secret);
    expect(technical).toContain('[REDACTED]');
    fireEvent.click(details.querySelector('summary')!);
    expect(details.open).toBe(true);
    fireEvent.click(within(details).getByRole('button', { name: common.errorFeedback.copyDetails }));
    await waitFor(() => expect(copy).toHaveBeenCalledTimes(1));
    const copied = copy.mock.calls[0]?.[0] ?? '';
    expect(copied).toBe(technical);
    for (const value of ['task-failed', 'GPU queue exhausted', 'network disconnected']) expect(copied).toContain(value);
    for (const secret of ['durable-secret', 'runtime-secret']) expect(copied).not.toContain(secret);
    fireEvent.click(within(dialog).getAllByRole('button', { name: 'Close' }).at(-1)!);
    await waitFor(() => expect(view.queryByRole('dialog', { name: 'Template task error' }) === null).toBe(true));
    expect(view.getByRole('dialog', { name: 'Template runs' }) === panel).toBe(true);
    expect(document.activeElement === failure).toBe(true);
    expect(view.getByRole('button', { name: 'Template runs' }).getAttribute('aria-expanded')).toBe('true');
  });

  test('keeps the original thrown action error available in shared technical details instead of losing it to fallback copy', async () => {
    const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
    restores.push(() => copy.mockRestore());
    const run = runFixture('running');
    const resume = mock(async () => { throw new Error('Resume transport failed: EPIPE; Authorization: Bearer action-secret'); });
    const port = createPort([run], { resume });
    port.snapshot.activities = { [run.request.id]: { state: 'paused', taskStatuses: {}, error: null } };
    const view = renderCenter(port);
    const panel = await openList(view);
    const trigger = rowFor(view, run).getByRole('button', { name: 'Resume' });
    trigger.focus();
    fireEvent.click(trigger);
    const dialog = await view.findByRole('dialog', { name: creativeStudio.templates.runCenter.actionError });
    expect(resume).toHaveBeenCalledWith(run.request.id);
    expect(within(dialog).getByText(creativeStudio.templates.runCenter.actionErrorSummary).closest('details')).toBeNull();
    const details = dialog.querySelector('details')!;
    expect(details.open).toBe(false);
    const technical = details.querySelector('pre')?.textContent ?? '';
    expect(technical).toContain('Error: Resume transport failed: EPIPE');
    expect(technical).toContain('Bearer [REDACTED]');
    expect(dialog.textContent).not.toContain('action-secret');
    fireEvent.click(details.querySelector('summary')!);
    fireEvent.click(within(details).getByRole('button', { name: common.errorFeedback.copyDetails }));
    await waitFor(() => expect(copy).toHaveBeenCalledWith(technical));
    fireEvent.click(within(dialog).getAllByRole('button', { name: 'Close' }).at(-1)!);
    await waitFor(() => expect(view.queryByRole('dialog', { name: creativeStudio.templates.runCenter.actionError }) === null).toBe(true));
    expect(view.getByRole('dialog', { name: 'Template runs' }) === panel).toBe(true);
    expect(document.activeElement === trigger).toBe(true);
  });

  test('preserves resume, cancel, retry, and prompt review through compact task actions', async () => {
    const paused = runFixture('running');
    const failed = runFixture('failed', IDS.idempotency);
    const reviewing = runFixture('awaiting-review', IDS.result2, true);
    reviewing.promptDrafts = [{
      id: IDS.draft1, templateId: IDS.template, runRequestId: reviewing.request.id, seriesIndex: 0,
      title: 'Series cover', prompt: 'A coherent product cover', status: 'pending-review',
      createdAt: 2000, reviewedAt: null, reviewNote: null,
    }];
    reviewing.record.promptDraftIds = [IDS.draft1];
    const port = createPort([paused, failed, reviewing]);
    port.snapshot.activities = { [paused.request.id]: { state: 'paused', taskStatuses: {}, error: null } };
    const view = renderCenter(port);
    await openList(view);
    fireEvent.click(rowFor(view, paused).getByRole('button', { name: 'Resume' }));
    await waitFor(() => expect(port.resume).toHaveBeenCalledWith(paused.request.id));
    fireEvent.click(rowFor(view, paused).getByRole('button', { name: 'More task actions' }));
    fireEvent.click(await view.findByRole('menuitem', { name: 'Cancel' }));
    await waitFor(() => expect(port.cancel).toHaveBeenCalledWith(paused.request.id));
    fireEvent.click(rowFor(view, failed).getByRole('button', { name: 'Run again' }));
    await waitFor(() => expect(port.retry).toHaveBeenCalledWith(failed));
    fireEvent.click(rowFor(view, reviewing).getByRole('button', { name: 'Review prompts' }));
    const dialog = await view.findByRole('dialog', { name: 'Review prompts · awaiting-review template' });
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Image 1 title' }), { target: { value: 'Revised cover' } });
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Image 1 prompt' }), { target: { value: 'A revised product cover' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Approve and start generation' }));
    await waitFor(() => expect(port.review).toHaveBeenCalledWith(reviewing.request.id, [{
      id: IDS.draft1, title: 'Revised cover', prompt: 'A revised product cover', reviewNote: null,
    }]));
  });

  test('disables rerunning when a recorded input asset was deleted', async () => {
    const run = runFixture('failed');
    run.request.inputs.push({ variableId: IDS.imageVariable, type: 'image', assetId: IDS.asset });
    const port = createPort([run], { assetReader: { get: async (id) => assetFixture(id, true) } });
    const view = renderCenter(port);
    await openList(view);
    await waitFor(() => expect((rowFor(view, run).getByRole('button', { name: 'Run again' }) as HTMLButtonElement).disabled).toBe(true));
    fireEvent.click(rowFor(view, run).getByRole('button', { name: 'Run again' }));
    expect(port.retry).not.toHaveBeenCalled();
    expect(view.queryByRole('img')).toBeNull();
  });

  test('opens all seven results, keeps deleted slots navigable, and restores the result trigger on close', async () => {
    const run = runFixture('succeeded', IDS.request, true);
    const ids = Array.from({ length: 7 }, (_, index) => `018f0000-0000-7000-8000-${String(20 + index).padStart(12, '0')}`);
    run.record.resultAssetIds = ids;
    const port = createPort([run], { assetReader: { get: async (id) => assetFixture(id, id === ids[1]) } });
    const view = renderCenter(port);
    const panel = await openList(view);
    expect(within(panel).getByText('7 images')).toBeTruthy();
    expect(within(panel).queryByRole('img')).toBeNull();
    const trigger = rowFor(view, run).getByRole('button', { name: 'Open result' });
    fireEvent.click(trigger);
    const dialog = await view.findByRole('dialog', { name: 'View image' });
    expect(dialog.className).toContain('nomifun-modal-fullscreen');
    await waitFor(() => expect(within(dialog).getByRole('img', { hidden: true }).getAttribute('src')).toBe(port.assetUrl(ids[0]!)));
    expect(within(dialog).getByText('1 / 7')).toBeTruthy();
    expect((within(dialog).getByRole('button', { name: 'Previous image' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Next image' }));
    await waitFor(() => expect(within(dialog).getByRole('alert').textContent).toContain('Asset deleted'));
    expect(within(dialog).getByText('2 / 7')).toBeTruthy();
    expect(within(dialog).queryByRole('img')).toBeNull();
    expect((within(dialog).getByRole('button', { name: 'Save image as' }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(dialog).getByRole('button', { name: 'Next image' }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Next image' }));
    await waitFor(() => expect(within(dialog).getByRole('img', { hidden: true }).getAttribute('src')).toBe(port.assetUrl(ids[2]!)));
    const image = within(dialog).getByRole('img', { hidden: true });
    Object.defineProperties(image, { naturalWidth: { value: 720 }, naturalHeight: { value: 1280 } });
    fireEvent.load(image);
    expect((within(dialog).getByRole('button', { name: 'Save image as' }) as HTMLButtonElement).disabled).toBe(false);
    const previewClose = within(dialog).getByRole('button', { name: 'Close image preview' });
    for (let index = 3; index < 7; index++) fireEvent.keyDown(previewClose, { key: 'ArrowRight' });
    expect(within(dialog).getByText('7 / 7')).toBeTruthy();
    expect(within(dialog).getByRole('img', { hidden: true }).getAttribute('src')).toBe(port.assetUrl(ids[6]!));
    expect((within(dialog).getByRole('button', { name: 'Next image' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(previewClose, { key: 'ArrowRight' });
    expect(within(dialog).getByText('7 / 7')).toBeTruthy();
    fireEvent.keyDown(previewClose, { key: 'ArrowLeft' });
    expect(within(dialog).getByText('6 / 7')).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close image preview' }));
    await waitFor(() => expect(view.queryByRole('dialog', { name: 'View image' }) === null).toBe(true));
    expect(view.getByRole('dialog', { name: 'Template runs' }) === panel).toBe(true);
    expect(document.activeElement === trigger).toBe(true);
  });
});
