import { describe, expect, test } from 'bun:test';
import {
  extractSalesDashboardEvents,
  mergeSalesAgentSnapshot,
  SALES_DASHBOARD_EVENT_PREFIX,
} from './salesAgentReport';
import { buildSalesTask, createEmptySalesWorkspace } from './salesWorkspace';

const submittedLine = `${SALES_DASHBOARD_EVENT_PREFIX}${JSON.stringify({
  event_id: 'task-1:example.jp:submitted',
  company_name: 'Example Japan',
  website: 'https://example.jp',
  country: '日本',
  industry: '生活杂货',
  status: 'submitted',
  fit_summary: '官网显示经营进口生活用品，符合目标客户条件。',
  evidence_url: 'https://example.jp/about',
  form_url: 'https://example.jp/contact',
  outreach_message: '您好，我们希望介绍小批量跨境供货服务。',
  restriction_summary: '未发现明确禁止销售联络的说明。',
  detail: '官网显示提交成功。',
  occurred_at: '2026-08-31T04:00:00.000Z',
})}`;

describe('sales Agent dashboard reports', () => {
  test('extracts valid single-line events and ignores partial streaming JSON', () => {
    const events = extractSalesDashboardEvents(`处理中\n${submittedLine}\n${SALES_DASHBOARD_EVENT_PREFIX}{"status":`);
    expect(events).toHaveLength(1);
    expect(events[0].companyName).toBe('Example Japan');
    expect(events[0].status).toBe('submitted');
    expect(events[0].outreachMessage.includes('跨境供货')).toBe(true);
  });

  test('upserts company content and does not duplicate a repeated terminal event', () => {
    const workspace = createEmptySalesWorkspace();
    workspace.tasks.push({
      ...buildSalesTask(
        {
          name: '日本市场开发',
          cadence: 'once',
          targetCount: 5,
          countries: ['日本'],
          scheduleTime: '09:00',
          notes: '',
        },
        'task-1',
        '2026-08-31T00:00:00.000Z'
      ),
      status: 'running',
      conversationId: 'conversation-1',
    });
    const events = extractSalesDashboardEvents(submittedLine);
    const snapshot = {
      taskId: 'task-1',
      taskStatus: 'completed' as const,
      agentSummary: '任务完成。',
      syncedAt: '2026-08-31T04:01:00.000Z',
      events,
    };

    const once = mergeSalesAgentSnapshot(workspace, snapshot);
    const twice = mergeSalesAgentSnapshot(once, snapshot);

    expect(twice.tasks[0].status).toBe('completed');
    expect(twice.companies).toHaveLength(1);
    expect(twice.companies[0].status).toBe('submitted');
    expect(twice.companies[0].contactFormUrl).toBe('https://example.jp/contact');
    expect(twice.companies[0].outreachMessage?.includes('跨境供货')).toBe(true);
    expect(twice.results).toHaveLength(1);
    expect(twice.results[0].outcome).toBe('submitted');
  });
});
