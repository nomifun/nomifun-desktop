import { Button, Select } from '@arco-design/web-react';
import { ChartHistogram, LinkOne, Refresh, Robot, Send, Shield } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { formatSalesDate, SalesEmptyState, SalesPageHeader, SalesSection, SalesStatusPill } from './SalesUi';

const cleanAgentSummary = (summary: string) =>
  summary
    .split(/\r?\n/)
    .filter((line) => !line.includes('SALES_DASHBOARD_EVENT '))
    .join('\n')
    .trim();

const SalesExecutionPage: React.FC = () => {
  const { workspace, syncAgentProgress, syncingAgentProgress } = useSalesWorkspace();
  const executionTasks = useMemo(
    () => workspace.tasks.filter(
      (task) => !task.archivedAt && (task.status !== 'draft' || task.conversationId)
    ),
    [workspace.tasks]
  );
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedTaskId = searchParams.get('task') ?? '';
  const [selectedTaskId, setSelectedTaskId] = useState(requestedTaskId || executionTasks[0]?.id || '');

  useEffect(() => {
    const requestedTask = executionTasks.find((task) => task.id === requestedTaskId);
    if (requestedTask && requestedTask.id !== selectedTaskId) {
      setSelectedTaskId(requestedTask.id);
      return;
    }
    if (!executionTasks.some((task) => task.id === selectedTaskId)) {
      setSelectedTaskId(executionTasks[0]?.id ?? '');
    }
  }, [executionTasks, requestedTaskId, selectedTaskId]);

  const selectTask = (taskId: string) => {
    setSelectedTaskId(taskId);
    const nextSearchParams = new URLSearchParams(searchParams);
    nextSearchParams.set('task', taskId);
    setSearchParams(nextSearchParams, { replace: true });
  };

  const task = executionTasks.find((item) => item.id === selectedTaskId) ?? executionTasks[0];
  const companies = task ? workspace.companies.filter((company) => company.taskId === task.id) : [];
  const results = task ? workspace.results.filter((result) => result.taskId === task.id) : [];
  const submitted = results.filter((result) => result.outcome === 'submitted').length;
  const skipped = results.filter((result) => result.outcome === 'skipped').length;
  const failed = results.filter((result) => result.outcome === 'failed').length;
  const processed = submitted + skipped + failed;
  const target = task?.targetCount ?? 0;
  const progress = target > 0 ? Math.min(100, Math.round((processed / target) * 100)) : 0;
  const summary = cleanAgentSummary(task?.agentSummary ?? '');

  return (
    <div className='sales-page-scroll'>
      <div className='sales-page sales-execution-page'>
        <SalesPageHeader
          title='执行看板'
          description='跟踪 Agent 的实时进度、自动提交结果、联络内容和失败原因。'
          action={
            <Button
              icon={<Refresh size={14} />}
              loading={syncingAgentProgress}
              disabled={!task?.conversationId}
              onClick={() => void syncAgentProgress()}
            >
              刷新进度
            </Button>
          }
        />

        {task ? (
          <>
            <SalesSection className='sales-run-overview'>
              <div className='sales-run-overview__header'>
                <div>
                  <div className='sales-run-overview__title'>
                    <h2>{task.name}</h2>
                    <SalesStatusPill status={task.status} />
                  </div>
                  <p>{task.countries.join('、')} · 目标 {task.targetCount} 家公司 · 合格后自动提交</p>
                </div>
                {executionTasks.length > 1 ? (
                  <Select
                    aria-label='选择销售任务'
                    value={task.id}
                    onChange={selectTask}
                    options={executionTasks.map((item) => ({ label: item.name, value: item.id }))}
                    style={{ width: 240 }}
                  />
                ) : null}
              </div>

              <div className='sales-run-progress' aria-label={`任务完成进度 ${progress}%`}>
                <div>
                  <span>任务完成进度</span>
                  <strong>{processed} / {target}</strong>
                </div>
                <div className='sales-run-progress__track'>
                  <span style={{ width: `${progress}%` }} />
                </div>
                <small>{progress}%</small>
              </div>

              <div className='sales-run-metrics' aria-label='执行数据概览'>
                <div><span>目标数量</span><strong>{target}</strong></div>
                <div><span>已发现</span><strong>{companies.length}</strong></div>
                <div><span>已自动发送</span><strong>{submitted}</strong></div>
                <div><span>不符合 / 跳过</span><strong>{skipped}</strong></div>
                <div><span>需要处理</span><strong>{failed}</strong></div>
              </div>
            </SalesSection>

            <div className='sales-execution-split'>
              <SalesSection title='Agent 当前状态' description='从关联的 NomiFun Agent 会话持续同步。'>
                <div className='sales-agent-state'>
                  <span className={`sales-agent-state__icon ${task.status === 'running' ? 'is-running' : ''}`}>
                    <Robot size={20} />
                  </span>
                  <div>
                    <strong>
                      {!task.conversationId
                        ? '旧任务未关联 Agent'
                        : task.status === 'running'
                          ? 'NomiFun Agent 正在执行任务'
                          : task.status === 'failed'
                            ? 'NomiFun Agent 本轮执行失败'
                            : 'NomiFun Agent 本轮执行已结束'}
                    </strong>
                    <span>
                      {!task.conversationId
                        ? '重新点击“立即执行”创建的新任务会自动同步'
                        : task.agentUpdatedAt
                          ? `最近同步 ${formatSalesDate(task.agentUpdatedAt)}`
                          : '等待首次同步'}
                    </span>
                  </div>
                  {task.conversationId ? (
                    <Link to={`/conversation/${task.conversationId}`}>
                      <Button size='small' type='secondary'>查看会话</Button>
                    </Link>
                  ) : null}
                </div>
                <div className='sales-agent-summary'>
                  <span>最新 Agent 输出</span>
                  <p>
                    {summary || (!task.conversationId
                      ? '这个任务创建于进度关联功能上线前，无法从旧会话回收结构化进度。'
                      : task.status === 'running'
                        ? '正在等待 NomiFun Agent 的首次回复；产生结构化公司记录后会显示在这里。'
                        : '本轮没有可显示的文字摘要。')}
                  </p>
                </div>
              </SalesSection>

              <SalesSection title='自动提交规则' description='无需逐家公司确认，但仍保留资格判断。'>
                <div className='sales-auto-policy'>
                  <Shield size={21} />
                  <div>
                    <strong>仅向合格公司发送</strong>
                    <p>官网证据充分、用途匹配，且没有明确禁止销售联络时才提交。</p>
                  </div>
                </div>
                <ul className='sales-policy-list'>
                  <li>每家公司最多提交一次</li>
                  <li>验证码、登录或结果不明确时停止并记录失败</li>
                  <li>完整发送内容、官网证据和表单地址都会留档</li>
                </ul>
              </SalesSection>
            </div>

            <SalesSection
              title='公司执行明细'
              description='查看每家公司的资格判断、发送内容和最终结果。'
              action={<span className='sales-record-count'>{companies.length} 家公司</span>}
            >
              {companies.length > 0 ? (
                <div className='sales-table-wrap'>
                  <table className='sales-table sales-execution-table'>
                    <thead>
                      <tr><th>公司</th><th>AI 判断</th><th>执行状态</th><th>联络内容 / 结果</th><th>更新时间</th></tr>
                    </thead>
                    <tbody>
                      {companies.map((company) => (
                        <tr key={company.id}>
                          <td>
                            <strong>{company.name}</strong>
                            <small>{company.country}{company.industry ? ` · ${company.industry}` : ''}</small>
                            {company.evidenceUrl ? (
                              <a className='sales-inline-link' href={company.evidenceUrl} target='_blank' rel='noreferrer'>
                                <LinkOne size={11} />官网证据
                              </a>
                            ) : null}
                          </td>
                          <td className='sales-table__summary'>{company.fitSummary || '分析中'}</td>
                          <td><SalesStatusPill status={company.status} /></td>
                          <td>
                            {company.outreachMessage ? (
                              <details className='sales-message-details'>
                                <summary><Send size={12} />查看发送内容</summary>
                                <pre>{company.outreachMessage}</pre>
                              </details>
                            ) : <span className='sales-muted'>暂无发送内容</span>}
                            {company.contactFormUrl ? (
                              <a className='sales-inline-link' href={company.contactFormUrl} target='_blank' rel='noreferrer'>
                                <LinkOne size={11} />联系表单
                              </a>
                            ) : null}
                          </td>
                          <td>{formatSalesDate(company.updatedAt)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ) : (
                <div className='sales-inline-dashboard-empty'>
                  <ChartHistogram size={24} />
                  <div>
                    <strong>
                      {!task.conversationId
                        ? '旧任务没有可同步的公司记录'
                        : task.status === 'running'
                          ? 'Agent 正在准备第一条公司记录'
                          : '尚未同步到结构化公司记录'}
                    </strong>
                    <span>
                      {task.conversationId
                        ? '你可以刷新进度；Agent 返回第一条结构化结果后会显示在这里。'
                        : '请在联络任务页面重新创建并立即执行。'}
                    </span>
                  </div>
                </div>
              )}
            </SalesSection>
          </>
        ) : (
          <SalesSection>
            <SalesEmptyState
              icon={<ChartHistogram size={25} />}
              title='还没有正在执行的任务'
              description='点击“立即执行”后，Agent 进度、自动提交内容和失败原因会显示在这里。'
              actionLabel='创建销售任务'
              actionTo='/sales/tasks'
            />
          </SalesSection>
        )}
      </div>
    </div>
  );
};

export default SalesExecutionPage;
