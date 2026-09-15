import FilePreview from '@/renderer/components/media/FilePreview';
import { CreationReferences } from '@/renderer/creation/CreationControls';
import { useCreationComposer } from '@/renderer/creation/CreationComposerContext';
import { filesForCreation } from '@/renderer/creation/types';
import type { FileSelectionItem } from '@/renderer/utils/file/fileSelection';
import styles from './ComposerAttachments.module.css';

export default function ComposerAttachments({ files, onRemoveFile, workspaceItems = [], onRemoveWorkspaceItem }: {
  files: readonly string[];
  onRemoveFile(path: string): void;
  workspaceItems?: readonly FileSelectionItem[];
  onRemoveWorkspaceItem?(path: string): void;
}) {
  const creation = useCreationComposer();
  const paths = [...new Set([...files, ...workspaceItems.map(item => typeof item === 'string' ? item : item.path)])];
  const activePaths = creation?.draft.mode ? filesForCreation(creation.draft.mode, paths) : paths;
  return <div className={styles.strip} role='list' aria-label='附件与引用' data-composer-attachments>
    {paths.map((path, index) => {
      const selection = workspaceItems.find(item => typeof item !== 'string' && item.path === path);
      return <FilePreview key={path} path={path} compact ordinal={index + 1} inactive={!activePaths.includes(path)} isDirectory={typeof selection === 'object' && !selection.isFile} onRemove={() => {
        if (files.includes(path)) onRemoveFile(path);
        onRemoveWorkspaceItem?.(path);
      }} />;
    })}
    <CreationReferences startIndex={paths.length} />
  </div>;
}
