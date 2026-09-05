import { describe, expect, test } from 'bun:test';
import {
  archiveSalesTask,
  buildSalesTask,
  createEmptySalesWorkspace,
  isCompanyProfileReady,
  parseSalesWorkspace,
} from './salesWorkspace';

describe('sales workspace state', () => {
  test('recovers safely from malformed persisted data', () => {
    expect(parseSalesWorkspace('{broken')).toEqual(createEmptySalesWorkspace());
    expect(parseSalesWorkspace(JSON.stringify({ version: 2 }))).toEqual(createEmptySalesWorkspace());
  });

  test('normalizes task input before it reaches the UI', () => {
    const task = buildSalesTask(
      {
        name: ' 日本市场 ',
        cadence: 'daily',
        targetCount: 5.4,
        countries: ['日本', ' 日本 ', '', '日本'],
        scheduleTime: '08:30',
        notes: ' 生活杂货 ',
      },
      'task-1',
      '2026-08-31T00:00:00.000Z'
    );

    expect(task.name).toBe('日本市场');
    expect(task.targetCount).toBe(5);
    expect(task.countries).toEqual(['日本']);
    expect(task.notes).toBe('生活杂货');
    expect(task.status).toBe('draft');
  });

  test('requires a useful company brief before marking onboarding complete', () => {
    const empty = createEmptySalesWorkspace().companyProfile;
    expect(isCompanyProfileReady(empty)).toBe(false);
    expect(
      isCompanyProfileReady({
        ...empty,
        companyName: 'Example Inc.',
        businessSummary: '生产环保生活杂货',
        valueProposition: '小批量跨境供货',
      })
    ).toBe(true);
  });

  test('archives a saved task without deleting companies or result records', () => {
    const workspace = createEmptySalesWorkspace();
    const task = buildSalesTask(
      {
        name: '日本市场',
        cadence: 'once',
        targetCount: 1,
        countries: ['日本'],
        scheduleTime: '09:00',
        notes: '',
      },
      'task-1',
      '2026-08-31T00:00:00.000Z'
    );
    workspace.tasks.push(task);
    workspace.companies.push({
      id: 'company-1',
      taskId: task.id,
      name: 'Example Co.',
      website: 'https://example.com',
      country: '日本',
      industry: '零售',
      fitSummary: '符合目标客户',
      status: 'submitted',
      updatedAt: '2026-08-31T01:00:00.000Z',
    });
    workspace.results.push({
      id: 'result-1',
      taskId: task.id,
      companyId: 'company-1',
      companyName: 'Example Co.',
      country: '日本',
      outcome: 'submitted',
      detail: '联系表单已提交',
      completedAt: '2026-08-31T01:00:00.000Z',
    });

    const archived = archiveSalesTask(workspace, task.id, '2026-08-31T02:00:00.000Z');

    expect(archived.tasks[0]?.archivedAt).toBe('2026-08-31T02:00:00.000Z');
    expect(archived.companies).toEqual(workspace.companies);
    expect(archived.results).toEqual(workspace.results);
  });
});
