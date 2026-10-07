import { afterEach, expect, mock, test } from 'bun:test';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IRequirement, RequirementStatus } from '@/common/adapter/ipcBridge';
import { parseRequirementId } from '@/common/types/ids';
import StatusPill from '../components/StatusPill';
import RequirementBoardView from './RequirementBoardView';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
afterEach(cleanup);
// Explicit service contract: active claims and terminal results cannot be
// manually overwritten; failed/needs_review may be requeued to pending.
const transitions: Array<[RequirementStatus, RequirementStatus[]]> = [
  ['pending', ['needs_review', 'done', 'failed', 'cancelled']],
  ['in_progress', []],
  ['needs_review', ['pending', 'done', 'failed', 'cancelled']],
  ['done', []],
  ['failed', ['pending']],
  ['cancelled', []],
];

test.each(transitions)('status picker for %s exposes only manual transitions', async (status, allowed) => {
  const change = mock();
  const v = render(<I18nextProvider i18n={i18n}><StatusPill status={status} onChange={change} /></I18nextProvider>);
  if (allowed.length === 0) {
    expect(v.queryByRole('button') === null).toBe(true);
  } else {
    fireEvent.click(v.getByRole('button'));
    const menu = await v.findByRole('menu');
    expect(within(menu).getAllByRole('menuitem').map((el) => el.textContent)).toEqual(allowed.map((next) => 'requirements.status.' + next));
    fireEvent.click(within(menu).getByText('requirements.status.' + allowed[0]));
    expect(change).toHaveBeenCalledWith(allowed[0]);
  }
});

const item = (status: RequirementStatus): IRequirement => ({
  requirement_id: parseRequirementId('019b0000-0000-7000-8000-000000000001'),
  display_no: 1, title: 'draggable requirement', content: '', tag: 'fixture', order_key: '', status,
  attempt_count: 0, created_by: 'user', created_at: 1, updated_at: 1,
});
function board(status: RequirementStatus) {
  const change = mock();
  const value = item(status);
  const v = render(<I18nextProvider i18n={i18n}><RequirementBoardView items={[value]} onOpenDetail={() => {}} onStatusChange={change} /></I18nextProvider>);
  const card = v.container.querySelector('.requirements-board-card')!;
  const columns = v.container.querySelectorAll('.requirements-board-column');
  const dataTransfer = { setData: mock(), getData: () => '', effectAllowed: '', dropEffect: '' };
  return { value, change, card, columns, dataTransfer };
}

test.each(transitions)('board drag from %s follows the service transition contract', (status, allowed) => {
  const v = board(status);
  transitions.forEach(([next], index) => {
    v.change.mockClear();
    fireEvent.dragStart(v.card, { dataTransfer: v.dataTransfer });
    fireEvent.drop(v.columns[index]!, { dataTransfer: v.dataTransfer });
    if (allowed.includes(next)) expect(v.change).toHaveBeenCalledWith(v.value.requirement_id, next);
    else expect(v.change).not.toHaveBeenCalled();
  });
});

test('a cancelled drag cannot supply a stale id to a later external drop', () => {
  const v = board('pending');
  fireEvent.dragStart(v.card, { dataTransfer: v.dataTransfer });
  fireEvent.dragEnd(v.card, { dataTransfer: v.dataTransfer });
  fireEvent.drop(v.columns[3]!, { dataTransfer: v.dataTransfer });
  expect(v.change).not.toHaveBeenCalled();
});
