import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';
import React from 'react';

type AtFileMenuProps = {
  activeIndex: number;
  emptyText: string;
  failure?: { message: string; retryLabel: string; onRetry: () => void };
  items: FileOrFolderItem[];
  label: string;
  loading: boolean;
  loadingText: string;
  onHoverItem: (index: number) => void;
  onSelectItem: (item: FileOrFolderItem) => void;
};

const AtFileMenu: React.FC<AtFileMenuProps> = ({
  activeIndex,
  emptyText,
  failure,
  items,
  label,
  loading,
  loadingText,
  onHoverItem,
  onSelectItem,
}) => {
  return (
    <div
      className='rounded-14px border border-solid overflow-hidden p-6px flex flex-col gap-2px'
      style={{
        borderColor: 'var(--color-border-2)',
        background: 'color-mix(in srgb, var(--color-bg-1) 94%, transparent)',
        backdropFilter: 'blur(14px) saturate(1.05)',
        WebkitBackdropFilter: 'blur(14px) saturate(1.05)',
      }}
      role={loading || failure ? 'group' : 'listbox'}
      aria-label={label}
    >
      {loading ? (
        <div role='status' className='px-12px py-10px text-12px text-t-secondary'>{loadingText}</div>
      ) : failure ? (
        <div role='alert' className='px-12px py-10px text-12px text-t-primary flex items-center justify-between gap-12px'>
          <span>{failure.message}</span>
          <button
            type='button'
            className='px-10px py-4px rounded-6px shrink-0 cursor-pointer border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-2)]'
            onMouseDown={(event) => { event.preventDefault(); }}
            onClick={failure.onRetry}
          >
            {failure.retryLabel}
          </button>
        </div>
      ) : items.length === 0 ? (
        <div className='px-12px py-10px text-12px text-t-secondary'>{emptyText}</div>
      ) : (
        items.map((item, index) => {
          const isActive = index === activeIndex;
          return (
            <div
              key={item.path}
              role='option'
              aria-selected={isActive}
              className='px-12px py-8px rounded-10px cursor-pointer transition-colors'
              style={{
                background: isActive ? 'var(--color-fill-2)' : 'transparent',
              }}
              onMouseEnter={() => {
                onHoverItem(index);
              }}
              onMouseDown={(event) => {
                event.preventDefault();
                onSelectItem(item);
              }}
            >
              <div className='text-13px font-medium text-t-primary'>{item.name}</div>
              <div className='text-12px text-t-secondary break-all'>{item.relativePath || item.path}</div>
            </div>
          );
        })
      )}
    </div>
  );
};

export default AtFileMenu;
