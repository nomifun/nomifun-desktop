import { Button } from '@arco-design/web-react';
import { Download, SalesReport } from '@icon-park/react';
import React from 'react';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { SalesEmptyState, SalesPageHeader, SalesSection } from './SalesUi';
import { downloadSalesResultsCsv } from './salesResultsCsv';

const outcomeLabel = { submitted: '已提交', skipped: '已跳过', failed: '失败' } as const;

const SalesResultsPage: React.FC = () => {
  const { workspace } = useSalesWorkspace();
  return (
    <div className='sales-page-scroll'>
      <div className='sales-page'>
        <SalesPageHeader
          title='结果记录'
          description='汇总成功提交、主动跳过和失败记录；删除已保存计划不会删除这里的数据。'
          action={
            <Button
              disabled={workspace.results.length === 0}
              icon={<Download size={14} />}
              onClick={() => downloadSalesResultsCsv(workspace.results)}
            >
              导出结果
            </Button>
          }
        />
        <SalesSection>
          {workspace.results.length === 0 ? (
            <SalesEmptyState
              icon={<SalesReport size={25} />}
              title='还没有执行结果'
              description='Agent 完成公司处理后，提交时间、结果和失败原因会记录在这里。'
            />
          ) : (
            <div className='sales-table-wrap'>
              <table className='sales-table'>
                <thead><tr><th>公司</th><th>国家</th><th>结果</th><th>说明</th><th>完成时间</th></tr></thead>
                <tbody>
                  {workspace.results.map((result) => (
                    <tr key={result.id}>
                      <td><strong>{result.companyName}</strong></td>
                      <td>{result.country}</td>
                      <td><span className={`sales-outcome sales-outcome--${result.outcome}`}>{outcomeLabel[result.outcome]}</span></td>
                      <td className='sales-table__summary'>{result.detail}</td>
                      <td>{new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(result.completedAt))}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </SalesSection>
      </div>
    </div>
  );
};

export default SalesResultsPage;
