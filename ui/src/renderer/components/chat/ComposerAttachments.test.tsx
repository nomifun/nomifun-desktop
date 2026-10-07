import '../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { CreationComposerContext } from '@/renderer/creation/CreationComposerContext';
import { emptyCreationDraft } from '@/renderer/creation/useCreationDraft';
import ComposerAttachments from './ComposerAttachments';

afterEach(cleanup);

test('uploads, workspace selections and asset references share one strip with deduplicated paths', async () => {
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ size: 8 } as Awaited<ReturnType<typeof ipcBridge.fs.getFileMetadata.invoke>>);
  const removedFiles: string[] = [];
  const removedWorkspace: string[] = [];
  const draft = emptyCreationDraft();
  draft.references = [{ asset_id: 'asset-cat', kind: 'image', role: 'reference', title: '资产库中的猫', url: '/cat.png' }];
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
      <CreationComposerContext.Provider value={{ draft, update: () => {}, setMode: () => {}, selectMode: () => {}, exit: () => {} }}>
        <ComposerAttachments files={['C:/notes.txt']} workspaceItems={['C:/notes.txt', 'C:/brief.pdf']} onRemoveFile={path => removedFiles.push(path)} onRemoveWorkspaceItem={path => removedWorkspace.push(path)} />
      </CreationComposerContext.Provider>
    </SWRConfig>);
    const strip = page.getByRole('list', { name: '附件与引用' });
    expect(within(strip).getAllByRole('listitem')).toHaveLength(3);
    expect(within(strip).getByText('notes.txt')).toBeTruthy();
    expect(within(strip).getByText('brief.pdf')).toBeTruthy();
    expect(within(strip).getByRole('button', { name: '查看图片：资产库中的猫' })).toBeTruthy();
    fireEvent.click(within(strip).getByRole('button', { name: '移除notes.txt' }));
    expect(removedFiles).toEqual(['C:/notes.txt']);
    expect(removedWorkspace).toEqual(['C:/notes.txt']);
    await waitFor(() => expect(metadata).toHaveBeenCalledTimes(2));
  } finally { metadata.mockRestore(); }
});
