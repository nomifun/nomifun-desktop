import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { INewAttachmentRef } from '@/common/adapter/ipcBridge';
import { FileService, type FileMetadata } from '@/renderer/services/FileService';
import AttachmentsField from './AttachmentsField';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });
const image = (name: string): FileMetadata => ({ name: name + '.png', path: '/' + name + '.png', size: 1, type: 'image/png', lastModified: 1 });
const ref = (name: string): INewAttachmentRef => ({ source_path: '/' + name + '.png', file_name: name + '.png' });
function fixture(initial: INewAttachmentRef[] = []) {
  const pending: Array<{ resolve: (files: FileMetadata[]) => void; reject: (error: unknown) => void }> = [];
  const upload = spyOn(FileService, 'processDroppedFiles').mockImplementation(() => new Promise((resolve, reject) => { pending.push({ resolve, reject }); }));
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue(image('preview'));
  const preview = spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue('');
  restore.push(...[upload, metadata, preview].map((spy) => () => spy.mockRestore()));
  const changed = mock((_refs: INewAttachmentRef[]) => {});
  const uploading = mock((_value: boolean) => {});
  function Host() {
    const [value, setValue] = useState(initial);
    return <AttachmentsField value={value} onChange={(next) => { changed(next); setValue(next); }} onUploadingChange={uploading} />;
  }
  const view = render(<I18nextProvider i18n={i18n}><Host /></I18nextProvider>);
  fireEvent.click(view.getByRole('button', { expanded: false }));
  const input = view.getByTestId('requirement-attachment-input');
  const files = [new File(['x'], 'upload.png', { type: 'image/png' })];
  return { ...view, pending, changed, uploading,
    select: () => fireEvent.change(input, { target: { files } }),
    drop: () => fireEvent.drop(input.parentElement!, { dataTransfer: { files } }),
  };
}

test('overlapping input and drop completions append to the latest controlled attachments', async () => {
  const v = fixture();
  v.select(); v.drop();
  await act(async () => { v.pending[1]!.resolve([image('drop')]); v.pending[0]!.resolve([image('input')]); });
  expect(v.changed).toHaveBeenLastCalledWith([ref('drop'), ref('input')]);
});

test('an upload cannot resurrect an attachment removed while it was pending', async () => {
  const v = fixture([ref('removed')]); v.select();
  fireEvent.click(v.container.querySelector('.i-icon-close')!);
  expect(v.changed).toHaveBeenLastCalledWith([]);
  await act(async () => { v.pending[0]!.resolve([image('new')]); });
  expect(v.changed).toHaveBeenLastCalledWith([ref('new')]);
});

test('uploading stays true until every input request settles, including empty results', async () => {
  const v = fixture(); v.select(); v.select();
  expect(v.uploading).toHaveBeenLastCalledWith(true);
  await act(async () => { v.pending[0]!.resolve([]); });
  expect(v.uploading).toHaveBeenLastCalledWith(true);
  await act(async () => { v.pending[1]!.resolve([image('last')]); });
  expect(v.uploading).toHaveBeenLastCalledWith(false);
});

test('input and drop completions from an unmounted field cannot modify its former host', async () => {
  const v = fixture(); v.select(); v.drop(); v.unmount();
  await act(async () => { v.pending[0]!.resolve([image('old-input')]); v.pending[1]!.resolve([image('old-drop')]); });
  expect(v.changed).not.toHaveBeenCalled();
});
