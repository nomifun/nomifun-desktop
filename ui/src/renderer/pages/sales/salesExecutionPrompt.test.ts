import { describe, expect, test } from 'bun:test';
import {
  buildSalesExecutionPrompt,
  SALES_OPERATOR_SKILL_NAME,
} from './salesExecutionPrompt';
import { createEmptySalesWorkspace } from './salesWorkspace';

describe('buildSalesExecutionPrompt', () => {
  test('carries the customer brief, task scope, skill and automatic submission boundary', () => {
    const profile = {
      ...createEmptySalesWorkspace().companyProfile,
      companyName: 'Example Trading',
      businessSummary: '出口环保生活杂货',
      valueProposition: '支持小批量跨境供货',
    };
    const prompt = buildSalesExecutionPrompt(profile, {
      name: '日本市场开发',
      cadence: 'once',
      targetCount: 8,
      countries: ['日本'],
      scheduleTime: '09:00',
      notes: '优先独立零售商',
    }, 'task-123');

    expect(prompt.includes(SALES_OPERATOR_SKILL_NAME)).toBe(true);
    expect(prompt.includes('Example Trading')).toBe(true);
    expect(prompt.includes('目标国家：日本')).toBe(true);
    expect(prompt.includes('目标公司数量：8')).toBe(true);
    expect(prompt.includes('任务 ID：task-123')).toBe(true);
    expect(prompt.includes('自动提交')).toBe(true);
    expect(prompt.includes('不需要再次询问我')).toBe(true);
    expect(prompt.includes('SALES_DASHBOARD_EVENT')).toBe(true);
  });
});
