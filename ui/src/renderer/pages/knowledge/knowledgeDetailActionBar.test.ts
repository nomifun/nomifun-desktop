import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const detailSource = readFileSync(new URL('./KnowledgeDetailPage/index.tsx', import.meta.url), 'utf8');

describe('Knowledge detail document action bar', () => {
  test('keeps the back link icon and label vertically centered as one row', () => {
    expect(detailSource.includes('knowledge-detail-back-link')).toBe(true);
    expect(detailSource.includes('knowledge-detail-back-icon')).toBe(true);
    expect(detailSource.includes('[&_svg]:block')).toBe(true);
    expect(detailSource.includes("<Left theme='outline' size='14' />\n          <span>")).toBe(false);
  });

  test('uses a soft borderless action bar for new and upload actions', () => {
    expect(detailSource.includes('knowledge-doc-actions')).toBe(true);
    expect(detailSource.includes('knowledge-doc-action')).toBe(true);
    expect(detailSource.includes('Bottom actions: new + upload */}\n                <div className=\'flex gap-7px mt-8px border-t')).toBe(false);
    expect(detailSource.includes('border-none bg-transparent')).toBe(true);
  });

  test('disables connector entries while Feishu knowledge creation is disabled', () => {
    expect(detailSource.includes('FEISHU_KNOWLEDGE_CREATION_ENABLED')).toBe(true);
    expect(detailSource.includes("disabled={!FEISHU_KNOWLEDGE_CREATION_ENABLED}")).toBe(true);
    expect(detailSource.includes("!FEISHU_KNOWLEDGE_CREATION_ENABLED && 'cursor-not-allowed opacity-50'")).toBe(true);
    expect(detailSource.includes("onClick={() => setConnectorVisible(true)}")).toBe(false);
    expect(detailSource.includes("onConnectorOpen={() => setConnectorVisible(true)}")).toBe(false);
  });
});
