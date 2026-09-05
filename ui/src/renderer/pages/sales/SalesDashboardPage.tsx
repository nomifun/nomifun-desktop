import { Button } from '@arco-design/web-react';
import { CheckOne, Plan, Right, Send, Target } from '@icon-park/react';
import React from 'react';
import { Link } from 'react-router-dom';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { formatSalesDate, SalesPageHeader, SalesSection, SalesStatusPill } from './SalesUi';
import { isCompanyProfileReady } from './salesWorkspace';

const SalesDashboardPage: React.FC = () => {
  const { workspace } = useSalesWorkspace();
  const profileReady = isCompanyProfileReady(workspace.companyProfile);
  const visibleTasks = workspace.tasks.filter((task) => !task.archivedAt);
  const submitted = workspace.results.filter((result) => result.outcome === 'submitted').length;
  const completedToday = workspace.results.filter((result) => {
    const completed = new Date(result.completedAt);
    const today = new Date();
    return completed.toDateString() === today.toDateString();
  }).length;

  return (
    <div className='sales-page-scroll'>
      <div className='sales-page'>
        <SalesPageHeader
          title='销售工作台'
          description='设置目标市场，跟踪公司研究、自动提交和执行结果。'
          action={
            <Link to='/sales/tasks'>
              <Button type='primary' icon={<Plan size={15} />}>
                新建销售任务
              </Button>
            </Link>
          }
        />

        <div className='sales-dashboard-grid'>
          <SalesSection className='sales-dashboard-overview'>
            <div className='sales-dashboard-overview__intro'>
              <div>
                <span className='sales-kicker'>今日工作</span>
                <h2>{profileReady ? `你好，${workspace.companyProfile.companyName}` : '先完善你的公司资料'}</h2>
                <p>
                  {profileReady
                    ? '系统会依据你的业务信息判断目标公司是否合适，并自动提交符合条件的官方联系表单。'
                    : 'AI 需要了解你的业务、价值主张和目标客户，才能准备可信的联络内容。'}
                </p>
              </div>
              {!profileReady ? (
                <Link to='/sales/company'>
                  <Button type='secondary'>
                    完善公司资料 <Right size={14} />
                  </Button>
                </Link>
              ) : null}
            </div>

            <div className='sales-summary-row' aria-label='销售任务概览'>
              <div>
                <span>已保存计划</span>
                <strong>{visibleTasks.length}</strong>
              </div>
              <div>
                <span>公司列表</span>
                <strong>{workspace.companies.length}</strong>
              </div>
              <div>
                <span>已自动发送</span>
                <strong>{submitted}</strong>
              </div>
              <div>
                <span>今日完成</span>
                <strong>{completedToday}</strong>
              </div>
            </div>
          </SalesSection>

          <SalesSection title='开始前检查' description='完成这两项后，任务才具备可靠的执行上下文。'>
            <div className='sales-readiness-list'>
              <Link to='/sales/company' className='sales-readiness-item'>
                <span className={profileReady ? 'is-complete' : ''}>
                  {profileReady ? <CheckOne size={17} /> : <Target size={17} />}
                </span>
                <div>
                  <strong>公司资料</strong>
                  <small>{profileReady ? '关键信息已准备' : '需要公司、业务和价值主张'}</small>
                </div>
                <Right size={14} />
              </Link>
              <Link to='/sales/tasks' className='sales-readiness-item'>
                <span className={visibleTasks.length > 0 ? 'is-complete' : ''}>
                  {visibleTasks.length > 0 ? <CheckOne size={17} /> : <Plan size={17} />}
                </span>
                <div>
                  <strong>销售任务</strong>
                  <small>{visibleTasks.length > 0 ? `已保存 ${visibleTasks.length} 个计划` : '设置国家和联系数量'}</small>
                </div>
                <Right size={14} />
              </Link>
            </div>
          </SalesSection>
        </div>

        <div className='sales-dashboard-lower'>
          <SalesSection title='最近任务' description='按创建时间显示最近保存的销售计划。'>
            {visibleTasks.length > 0 ? (
              <div className='sales-task-compact-list'>
                {visibleTasks.slice(0, 4).map((task) => (
                  <div key={task.id} className='sales-task-compact-row'>
                    <div>
                      <strong>{task.name}</strong>
                      <span>{task.countries.join('、')} · {task.targetCount} 家公司</span>
                    </div>
                    <div>
                      <SalesStatusPill status={task.status} />
                      <time>{formatSalesDate(task.createdAt)}</time>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className='sales-inline-empty'>
                <Plan size={20} />
                <div>
                  <strong>还没有销售计划</strong>
                  <span>创建第一个任务，设置目标国家和公司数量。</span>
                </div>
              </div>
            )}
          </SalesSection>

          <SalesSection title='自动提交边界' description='自动发送不等于降低资格标准。'>
            <div className='sales-safety-note'>
              <Send size={22} />
              <div>
                <strong>合格后自动发送并留档</strong>
                <p>Agent 会先核实官网、适配度和销售限制；成功、跳过和失败结果都会进入执行看板。</p>
              </div>
            </div>
          </SalesSection>
        </div>
      </div>
    </div>
  );
};

export default SalesDashboardPage;
