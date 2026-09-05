import type { SalesResult } from './salesWorkspace';

const escapeCsvCell = (value: string) => `"${value.replaceAll('"', '""')}"`;

export const createSalesResultsCsv = (results: SalesResult[]) => {
  const rows = [
    ['公司', '国家', '结果', '说明', '完成时间'],
    ...results.map((result) => [
      result.companyName,
      result.country,
      result.outcome === 'submitted' ? '已提交' : result.outcome === 'skipped' ? '已跳过' : '失败',
      result.detail,
      result.completedAt,
    ]),
  ];

  return `\uFEFF${rows.map((row) => row.map(escapeCsvCell).join(',')).join('\r\n')}`;
};

export const downloadSalesResultsCsv = (results: SalesResult[]) => {
  const blob = new Blob([createSalesResultsCsv(results)], { type: 'text/csv;charset=utf-8' });
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = href;
  anchor.download = `销售联络结果-${new Date().toISOString().slice(0, 10)}.csv`;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(href);
};
