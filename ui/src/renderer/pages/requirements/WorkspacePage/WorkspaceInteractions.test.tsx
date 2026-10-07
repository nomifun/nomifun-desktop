import { afterEach, expect, mock, test } from 'bun:test';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IRequirement } from '@/common/adapter/ipcBridge';
import { parseRequirementId } from '@/common/types/ids';
import RequirementListRow from './RequirementListRow';
import RequirementFilters from './RequirementFilters';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
afterEach(cleanup);

const item: IRequirement = {
  requirement_id: parseRequirementId('019b0000-0000-7000-8000-000000000001'),
  display_no: 1,
  title: 'requirement',
  content: '',
  tag: 'fixture',
  order_key: '',
  status: 'pending',
  attempt_count: 0,
  created_by: 'user',
  created_at: 1,
  updated_at: 1,
};

function rowFixture() {
  const detail = mock();
  const edit = mock();
  const remove = mock();
  const view = render(
    <I18nextProvider i18n={i18n}>
      <RequirementListRow
        item={item}
        selected={false}
        onToggleSelect={() => {}}
        onOpenDetail={detail}
        onStatusChange={() => {}}
        onEdit={edit}
        onDelete={remove}
      />
    </I18nextProvider>
  );
  return { view, detail, edit, remove };
}

test('child Enter belongs to its control while row Enter still opens details', () => {
  const { view, detail, edit } = rowFixture();
  fireEvent.keyDown(view.getByLabelText('requirements.actions.edit'), { key: 'Enter' });
  fireEvent.keyDown(view.getByRole('checkbox'), { key: 'Enter' });
  const childOpens = detail.mock.calls.length;
  detail.mockClear();
  fireEvent.keyDown(view.container.querySelector('.requirements-list-row')!, { key: 'Enter' });
  expect(edit).toHaveBeenCalledWith(item.requirement_id);
  expect(childOpens).toBe(0);
  expect(detail).toHaveBeenCalledTimes(1);
});

test('delete Enter opens confirmation without opening details or deleting early', async () => {
  const { view, detail, remove } = rowFixture();
  fireEvent.keyDown(view.getByLabelText('requirements.actions.delete'), { key: 'Enter' });
  await act(async () => {});
  expect(detail).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
  expect(view.queryByText('requirements.actions.deleteConfirm')).not.toBeNull();
  fireEvent.click(view.baseElement.querySelector('.arco-popconfirm-btn .arco-btn-primary')!);
  await act(async () => {});
  expect(remove).toHaveBeenCalledWith(item.requirement_id);
  expect(detail).not.toHaveBeenCalled();
});

test('literal sentinel and prefixed tags remain distinct from the all-tags action', async () => {
  const changed = mock();
  const tags = ['__all_tags__', 'tag:__all_tags__', '客户 🌱'];
  const tagOptions = tags.map(tag => ({
    tag, pending: 1, in_progress: 0, done: 0, failed: 0, cancelled: 0,
    needs_review: 0, total: 1, paused: false,
  }));
  const view = render(
    <I18nextProvider i18n={i18n}>
      <RequirementFilters
        search=''
        order='desc'
        selectedCount={0}
        tagOptions={tagOptions}
        onTagChange={changed}
        onStatusChange={() => {}}
        onSearchChange={() => {}}
        onOrderByChange={() => {}}
        onOrderChange={() => {}}
        onBatchDelete={() => {}}
      />
    </I18nextProvider>
  );
  for (const tag of [...tags, undefined]) {
    fireEvent.click(view.getByRole('button', { name: 'requirements.columns.tag' }));
    const menu = await view.findByRole('menu');
    fireEvent.click(within(menu).getByText(tag === undefined ? 'requirements.allTags' : tag + ' (0/1)'));
    await act(async () => {});
    expect(changed).toHaveBeenLastCalledWith(tag);
  }
});
