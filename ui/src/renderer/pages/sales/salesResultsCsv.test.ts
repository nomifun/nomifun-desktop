import { describe, expect, test } from 'bun:test';
import { createSalesResultsCsv } from './salesResultsCsv';

describe('createSalesResultsCsv', () => {
  test('exports Chinese labels and safely escapes spreadsheet cells', () => {
    const csv = createSalesResultsCsv([
      {
        id: 'result-1',
        companyId: 'company-1',
        companyName: 'Example, "Ltd"',
        country: '日本',
        outcome: 'submitted',
        detail: '提交成功',
        completedAt: '2026-08-31T09:00:00.000Z',
      },
    ]);

    expect(csv.startsWith('\uFEFF"公司","国家","结果"')).toBe(true);
    expect(csv.includes('"Example, ""Ltd"""')).toBe(true);
    expect(csv.includes('"已提交"')).toBe(true);
  });
});
