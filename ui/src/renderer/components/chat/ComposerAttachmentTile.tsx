import type { ReactNode } from 'react';
import { CloseOne } from '@icon-park/react';
import styles from './ComposerAttachments.module.css';

export default function ComposerAttachmentTile({ title, detail = title, ordinal, inactive, onRemove, children }: {
  title: string;
  detail?: string;
  ordinal?: number;
  inactive?: boolean;
  onRemove?: () => void;
  children: ReactNode;
}) {
  return <div role='listitem' className={styles.item} data-inactive={inactive || undefined}>
    <div className={styles.preview} title={detail}>
      {children}
      {ordinal !== undefined && <strong aria-hidden='true'>{ordinal}</strong>}
    </div>
    <span className={styles.name} title={detail}>{title}</span>
    {onRemove && <button type='button' className={styles.remove} aria-label={`移除${title}`} title='移除附件' onClick={onRemove}>
      <CloseOne theme='two-tone' size={12} strokeWidth={3} fill={['currentColor', 'var(--color-bg-popup)']} />
    </button>}
  </div>;
}
