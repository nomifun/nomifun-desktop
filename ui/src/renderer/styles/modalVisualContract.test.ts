import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const mainSource = readFileSync(new URL('../main.tsx', import.meta.url), 'utf8');
const contractCss = readFileSync(new URL('./modal-contract.css', import.meta.url), 'utf8');
const nomiModalSource = readFileSync(new URL('../components/base/NomiModal.tsx', import.meta.url), 'utf8');
const createTextSource = readFileSync(
  new URL('../pages/workshop/assets/CreateTextAssetModal.tsx', import.meta.url),
  'utf8'
);
const createCsStyles = readFileSync(
  new URL('../pages/customerService/CreateCsAgentModal.module.css', import.meta.url),
  'utf8'
);
const retrievalStyles = readFileSync(
  new URL('../pages/knowledge/KnowledgeRetrievalSettingsModal.module.css', import.meta.url),
  'utf8'
);
const createTaskSource = readFileSync(
  new URL('../pages/cron/ScheduledTasksPage/CreateTaskDialog.tsx', import.meta.url),
  'utf8'
);

describe('Global modal visual contract', () => {
  test('loads after theme styles so modal geometry stays consistent across themes', () => {
    const themesImport = mainSource.indexOf("import './styles/themes/index.css'");
    const contractImport = mainSource.indexOf("import './styles/modal-contract.css'");

    expect(themesImport).toBeGreaterThan(-1);
    expect(contractImport).toBeGreaterThan(themesImport);
  });

  test('keeps controls rounded without exceeding the modal corner radius', () => {
    const modalRadius = Number(contractCss.match(/--nomi-modal-radius:\s*(\d+)px/)?.[1]);
    const controlRadius = Number(contractCss.match(/--nomi-modal-control-radius:\s*(\d+)px/)?.[1]);

    expect(modalRadius).toBe(14);
    expect(controlRadius).toBe(8);
    expect(controlRadius).toBeLessThanOrEqual(modalRadius);
    expect(createCsStyles.includes('border-radius: 999px')).toBe(false);
  });

  test('uses compact modal chrome and compact form rhythm', () => {
    expect(contractCss.includes('--nomi-modal-inline-padding: 14px')).toBe(true);
    expect(contractCss.includes('--nomi-modal-block-padding: 8px')).toBe(true);
    expect(contractCss.includes('height: 40px')).toBe(true);
    expect(contractCss.includes('padding: var(--nomi-modal-block-padding) var(--nomi-modal-inline-padding)')).toBe(true);
    expect(contractCss.includes('.arco-modal .arco-form-item')).toBe(true);
    expect(contractCss.includes('margin-bottom: 12px')).toBe(true);
  });

  test('separates every populated footer from modal content with the same divider', () => {
    expect(contractCss.includes('.arco-modal .arco-modal-footer')).toBe(true);
    expect(contractCss.includes('border-top: 1px solid var(--border-base)')).toBe(true);
    expect(contractCss.includes('.nomifun-modal .nomifun-modal-footer:not(:empty)')).toBe(true);
    expect(nomiModalSource.includes("'nomifun-modal-footer flex-shrink-0 bg-transparent'")).toBe(true);
    expect(nomiModalSource.includes("className='flex justify-end gap-8px'")).toBe(true);
  });

  test('covers focus-lock wrapped modal sections without adding another inner layout layer', () => {
    expect(contractCss.includes('.arco-modal .arco-modal-header')).toBe(true);
    expect(contractCss.includes('.arco-modal .arco-modal-content')).toBe(true);
    expect(contractCss.includes('.arco-modal > .arco-modal-content')).toBe(false);
    expect(contractCss.includes('background-color: var(--dialog-fill-0)')).toBe(true);
    expect(contractCss.includes('.nomifun-modal-layout-header')).toBe(false);
  });

  test('shows the modal radius through focus-lock content without clipping child popups', () => {
    expect(contractCss.includes('.arco-modal > [data-focus-lock-disabled]')).toBe(true);
    expect(contractCss.includes('.arco-modal .arco-modal-content:first-child')).toBe(true);
    expect(contractCss.includes('.arco-modal .arco-modal-content:last-of-type')).toBe(true);
    expect(contractCss.includes('border-top-left-radius: inherit')).toBe(true);
    expect(contractCss.includes('border-bottom-right-radius: inherit')).toBe(true);
    expect(contractCss.includes('background-clip: padding-box')).toBe(true);
    expect(contractCss.includes('overflow: hidden')).toBe(false);
  });

  test('gives modal fields a visible outline and a surface close to the dialog background', () => {
    expect(contractCss.includes('var(--dialog-fill-0) 94%')).toBe(true);
    expect(contractCss.includes('border-color: var(--color-border-3) !important')).toBe(true);
    expect(contractCss.includes('.arco-input-inner-wrapper')).toBe(true);
    expect(contractCss.includes('.arco-select-view')).toBe(true);
    expect(contractCss.includes('.arco-textarea')).toBe(true);
  });

  test('also tightens custom NomiModal and the text-asset form shown in the reference', () => {
    expect(nomiModalSource.includes('pb-8px')).toBe(true);
    expect(nomiModalSource.includes('text-16px')).toBe(true);
    expect(createTextSource.includes("className='flex flex-col gap-10px'")).toBe(true);
    expect(createTextSource.includes('autoSize={{ minRows: 4, maxRows: 14 }}')).toBe(true);
  });

  test('keeps local modal shells on the shared compact inset without double-padding scroll content', () => {
    expect(createCsStyles.includes('padding: 0 var(--nomi-modal-inline-padding)')).toBe(true);
    expect(retrievalStyles.includes('var(--nomi-modal-block-padding) var(--nomi-modal-inline-padding)')).toBe(true);
    expect(createTaskSource.includes("className='overflow-y-auto pb-4px max-h-[min(68vh,640px)]'")).toBe(true);
    expect(createTaskSource.includes('overflow-y-auto px-24px')).toBe(false);
  });
});
