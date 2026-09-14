import { BuildingOne, LinkOne, Search } from '@icon-park/react';
import React from 'react';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { SalesEmptyState, SalesPageHeader, SalesSection, SalesStatusPill } from './SalesUi';

const SalesCompaniesPage: React.FC = () => {
  const { workspace } = useSalesWorkspace();
  return (
    <div className='sales-page-scroll'>
      <div className='sales-page'>
        <SalesPageHeader title='公司列表' description='查看 AI 找到的公司、官网证据、适配分析和当前处理状态。' />
        <SalesSection>
          {workspace.companies.length === 0 ? (
            <SalesEmptyState
              icon={<Search size={25} />}
              title='还没有目标公司'
              description='保存销售任务并启动自动执行后，经过官网核实的公司会出现在这里。'
              actionLabel='创建销售任务'
              actionTo='/sales/tasks'
            />
          ) : (
            <div className='sales-table-wrap'>
              <table className='sales-table'>
                <thead><tr><th>公司</th><th>国家 / 行业</th><th>AI 分析</th><th>状态</th><th>更新时间</th></tr></thead>
                <tbody>
                  {workspace.companies.map((company) => (
                    <tr key={company.id}>
                      <td>
                        <div className='sales-company-cell'><BuildingOne size={16} /><div><strong>{company.name}</strong><a href={company.website} target='_blank' rel='noreferrer'><LinkOne size={12} />{company.website}</a></div></div>
                      </td>
                      <td>{company.country}<small>{company.industry || '行业待确认'}</small></td>
                      <td className='sales-table__summary'>{company.fitSummary || '分析中'}</td>
                      <td><SalesStatusPill status={company.status} /></td>
                      <td>{new Intl.DateTimeFormat('zh-CN').format(new Date(company.updatedAt))}</td>
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

export default SalesCompaniesPage;

