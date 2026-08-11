import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');
// The toolbar's chrome (one-sided divider, 28px square icon buttons) is defined
// app-wide, so the hooks the component relies on are guarded against removal.
const arcoOverrides = readFileSync(new URL('../../styles/arco-override.css', import.meta.url), 'utf8');

describe('Knowledge detail document action bar', () => {
  test('keeps the back link icon and label vertically centered as one row', () => {
    expect(detailSource.includes('knowledge-detail-back-link')).toBe(true);
    expect(detailSource.includes('knowledge-detail-back-icon')).toBe(true);
    expect(detailSource.includes('[&_svg]:block')).toBe(true);
    expect(detailSource.includes("<Left theme='outline' size='14' />\n          <span>")).toBe(false);
  });

  test('uses a borderless icon-first toolbar for the document actions', () => {
    // The bar is `knowledge-doc-toolbar`: a single transparent row whose only
    // edge is the shared bottom divider. It replaced the old segmented
    // `knowledge-doc-actions` pill group of text buttons.
    expect(detailSource.includes('knowledge-doc-toolbar')).toBe(true);
    expect(detailSource.includes('knowledge-doc-actions')).toBe(false);
    expect(detailSource.includes("className='knowledge-doc-divider-bottom knowledge-doc-toolbar")).toBe(true);
    expect(detailSource.includes('knowledge-doc-toolbar flex h-42px shrink-0 items-center')).toBe(true);
    expect(detailSource.includes('bg-transparent')).toBe(true);
    expect(detailSource.includes("Bottom actions: new + upload */}\n                <div className='flex gap-7px mt-8px border-t")).toBe(false);

    // Every action is an icon-only control that keeps its label reachable via a
    // tooltip and an aria-label, through one shared primitive.
    expect(detailSource.includes('const KnowledgeIconButton')).toBe(true);
    expect(detailSource.includes("className='knowledge-doc-icon-button'")).toBe(true);
    expect(detailSource.includes('<Tooltip content={label} position={tooltipPosition} mini>')).toBe(true);
    expect(detailSource.includes('aria-label={label}')).toBe(true);

    // The chrome those hooks depend on must still exist.
    expect(arcoOverrides.includes('.knowledge-doc-divider-bottom {')).toBe(true);
    expect(arcoOverrides.includes('.knowledge-doc-icon-button.arco-btn {')).toBe(true);
  });

  test('places document actions above document search and includes folder creation', () => {
    const actionsIndex = detailSource.indexOf('knowledge-doc-toolbar');
    const searchIndex = detailSource.indexOf('knowledge-doc-search');
    expect(actionsIndex).toBeGreaterThan(-1);
    expect(searchIndex).toBeGreaterThan(-1);
    expect(actionsIndex).toBeLessThan(searchIndex);
    expect(detailSource.includes('openNewFileModal')).toBe(true);
    expect(detailSource.includes('openNewFolderModal')).toBe(true);
    expect(detailSource.includes('FolderPlus')).toBe(true);

    // Creation actions sit at the start of the bar; the view controls are
    // pushed to its trailing edge.
    const toolbarIndex = actionsIndex;
    const newFolderIndex = detailSource.indexOf('openNewFolderModal', toolbarIndex);
    const trailingGroupIndex = detailSource.indexOf("className='ml-auto flex items-center gap-2px'", toolbarIndex);
    expect(trailingGroupIndex).toBeGreaterThan(-1);
    expect(newFolderIndex).toBeLessThan(trailingGroupIndex);
    expect(trailingGroupIndex).toBeLessThan(searchIndex);
  });

  test('uses compact per-node menus instead of inline delete text in the document tree', () => {
    expect(detailSource.includes('knowledge-tree-node-row')).toBe(true);
    expect(detailSource.includes('knowledge-tree-node-name')).toBe(true);
    expect(detailSource.includes('knowledge-tree-node-action')).toBe(true);
    expect(detailSource.includes('knowledge-tree-node-more')).toBe(true);
    expect(detailSource.includes('handleTreeNodeMenuClick')).toBe(true);
    expect(detailSource.includes("key='new-file'")).toBe(true);
    expect(detailSource.includes("key='new-folder'")).toBe(true);
    expect(detailSource.includes("key='rename'")).toBe(true);
    expect(detailSource.includes("key='delete'")).toBe(true);
    expect(detailSource.includes('deleteFolderWarning')).toBe(true);
    expect(detailSource.includes("className='!hidden group-hover:!inline-flex shrink-0'")).toBe(false);
  });

  test('right-aligns tree row actions and reveals them only for the active row', () => {
    expect(detailSource.includes('knowledge-doc-tree')).toBe(true);
    expect(detailSource.includes('[&_.arco-tree-node-title-wrapper]:flex')).toBe(true);
    expect(detailSource.includes('[&_.arco-tree-node-title]:flex-1')).toBe(true);
    expect(detailSource.includes('knowledge-tree-node-row group flex w-full')).toBe(true);
    // Right-aligned and occupying a reserved fixed-width slot so revealing the
    // menu never reflows the file name. The exact width is a styling detail.
    expect(/knowledge-tree-node-action ml-auto w-\d+px grid shrink-0 place-items-center/.test(detailSource)).toBe(true);
    expect(detailSource.includes('opacity-0')).toBe(true);
    expect(detailSource.includes('group-hover:opacity-100')).toBe(true);
    expect(detailSource.includes('focus-within:opacity-100')).toBe(true);
    expect(detailSource.includes("aria-label={t('common.more'")).toBe(true);
  });

  test('carries no Feishu connector UI (removed integration; only the create-flow placeholder remains)', () => {
    expect(detailSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(false);
    expect(detailSource.includes('KnowledgeConnectorDrawer')).toBe(false);
    expect(detailSource.includes('setConnectorVisible')).toBe(false);
    expect(detailSource.includes('syncSource')).toBe(false);
  });

  test('uses theme-aware contrast for detail badges, active tabs, and settings fields', () => {
    expect(detailSource.includes('knowledge-detail-soft-active')).toBe(true);
    expect(detailSource.includes('knowledge-detail-kind-badge')).toBe(true);
    expect(detailSource.includes('knowledge-detail-user-tag')).toBe(true);
    expect(detailSource.includes('knowledge-detail-add-tag')).toBe(true);
    expect(detailSource.includes('knowledge-detail-tabs')).toBe(true);
    expect(detailSource.includes('knowledge-detail-settings-input')).toBe(true);
    expect(detailSource.includes('knowledge-settings-danger-section')).toBe(true);
    expect(detailSource.includes("textClass: 'text-[rgb(var(--primary-5))]'")).toBe(false);
    expect(detailSource.includes("textClass: 'text-[rgb(var(--success-5))]'")).toBe(false);
    expect(detailSource.includes("textClass: 'text-[rgb(var(--warning-5))]'")).toBe(false);
    expect(detailSource.includes('!bg-primary-1 !text-primary-6 font-600')).toBe(false);
  });
});
