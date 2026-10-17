import { expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { ICronJob, ICronJobRunStatus } from '@/common/adapter/ipcBridge';
import cron from '@/renderer/services/i18n/locales/zh-CN/cron.json';
import CronStatusTag from './CronStatusTag';

const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { cron } } } });

test.each([
  ['input_required', '需要你操作'],
  ['cancelled', '已取消'],
] as const)('%s is a nonfailure Cron outcome', (status, label) => {
  const job = { enabled: true, state: { last_status: status } } as ICronJob;
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><CronStatusTag job={job} /></I18nextProvider>);
  expect(html).toContain(label);
  expect(html).not.toContain('执行出错');
  expect(html).not.toContain('bg-danger');
});

test('a real Cron failure keeps its error indication', () => {
  const job = { enabled: true, state: { last_status: 'error' as ICronJobRunStatus } } as ICronJob;
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><CronStatusTag job={job} /></I18nextProvider>);
  expect(html).toContain('执行出错');
  expect(html).toContain('bg-danger');
});
