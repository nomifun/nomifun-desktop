import { Button, Input, InputNumber, InputTag, Popconfirm, Select } from '@arco-design/web-react';
import { Delete, Down, Plan, PlayOne, Plus, Right, Up } from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import { useNomiQuickStart } from '@/renderer/hooks/agent/useNomiQuickStart';
import { PRESET_CATALOG_SWR_KEY, fetchPresetCatalog } from '@/renderer/hooks/preset/presetCatalog';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useSalesWorkspace } from './SalesWorkspaceContext';
import { formatSalesDate, SalesPageHeader, SalesSection, SalesStatusPill } from './SalesUi';
import { buildSalesExecutionPrompt, SALES_OPERATOR_PRESET_NAME } from './salesExecutionPrompt';
import { isCompanyProfileReady, type NewSalesTask, type SalesTaskCadence } from './salesWorkspace';

const { TextArea } = Input;

const initialTask: NewSalesTask = {
  name: '',
  cadence: 'once',
  targetCount: 10,
  countries: [],
  scheduleTime: '09:00',
  notes: '',
};

const SalesTasksPage: React.FC = () => {
  const { workspace, addTask, updateTaskStatus, linkTaskConversation, removeTask } = useSalesWorkspace();
  const { start: startNomi, canStart } = useNomiQuickStart();
  const {
    data: presets = [],
    error: presetError,
    isLoading: presetLoading,
    mutate: refreshPresetCatalog,
  } = useSWR(PRESET_CATALOG_SWR_KEY, fetchPresetCatalog);
  const [draft, setDraft] = useState<NewSalesTask>(initialTask);
  const [executing, setExecuting] = useState(false);
  const [expandedTaskIds, setExpandedTaskIds] = useState<string[]>([]);
  const [message, contextHolder] = useArcoMessage({ maxCount: 2 });
  const profileReady = isCompanyProfileReady(workspace.companyProfile);
  const visibleTasks = workspace.tasks.filter((task) => !task.archivedAt);
  const salesPreset = useMemo(
    () => presets.find((preset) => preset.name === SALES_OPERATOR_PRESET_NAME && preset.enabled),
    [presets]
  );
  const agentReady = canStart && Boolean(salesPreset);
  const agentStatusReason = presetLoading
    ? '正在检查销售 Agent…'
    : presetError
      ? '无法读取 Agent 设定'
      : !salesPreset
        ? `未找到已启用的“${SALES_OPERATOR_PRESET_NAME}”设定`
        : !canStart
          ? '请先选择一个可用的 Chat 模型'
          : 'NomiFun Agent、销售 Skill 和当前模型已就绪';

  const validateDraft = () => {
    if (!profileReady) {
      message.warning('请先完善公司资料，再创建销售任务。');
      return false;
    }
    if (!draft.name.trim() || draft.countries.length === 0) {
      message.warning('请填写任务名称并至少选择一个国家。');
      return false;
    }
    return true;
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!validateDraft()) return;
    addTask(draft);
    setDraft(initialTask);
    message.success('销售计划已保存。');
  };

  const executeNow = async () => {
    if (!validateDraft() || executing) return;
    if (!agentReady || !salesPreset) {
      message.warning(agentStatusReason);
      return;
    }

    setExecuting(true);
    const taskId = addTask(draft, 'running');
    try {
      const started = await startNomi({
        name: draft.name.trim(),
        prompt: buildSalesExecutionPrompt(workspace.companyProfile, draft, taskId),
        presetId: salesPreset.preset_id,
        onCreated: (conversationId) => linkTaskConversation(taskId, conversationId),
      });
      if (!started) {
        updateTaskStatus(taskId, 'failed');
      } else {
        setDraft(initialTask);
        message.success('任务已交给 NomiFun Agent，执行进度会自动同步。');
      }
    } catch {
      updateTaskStatus(taskId, 'failed');
      message.error('NomiFun Agent 任务启动失败。');
    } finally {
      setExecuting(false);
    }
  };

  const toggleTaskCompanies = (taskId: string) => {
    setExpandedTaskIds((current) =>
      current.includes(taskId) ? current.filter((id) => id !== taskId) : [...current, taskId]
    );
  };

  const deleteSavedTask = (taskId: string, hasExecution: boolean) => {
    removeTask(taskId);
    setExpandedTaskIds((current) => current.filter((id) => id !== taskId));
    message.success(
      hasExecution
        ? '计划已移除；Agent 会继续执行，结果记录仍会保留。'
        : '计划已删除。'
    );
  };

  return (
    <div className='sales-page-scroll'>
      {contextHolder}
      <div className='sales-page'>
        <SalesPageHeader title='销售任务' description='设置目标国家、联系数量和执行频率，可以保存为计划或立即交给 Agent 执行。' />

        <div className={`sales-runtime-strip ${agentReady ? 'is-ready' : 'is-pending'}`}>
          <span className='sales-runtime-strip__dot' aria-hidden='true' />
          <div>
            <strong>{agentReady ? 'NomiFun Agent 已就绪' : 'NomiFun Agent 尚未就绪'}</strong>
            <span>{agentStatusReason}</span>
          </div>
          <Button size='small' loading={presetLoading} onClick={() => void refreshPresetCatalog()}>
            重新检查
          </Button>
        </div>

        {!profileReady ? (
          <div className='sales-prerequisite-banner'>
            <div>
              <strong>创建任务前需要公司资料</strong>
              <span>至少填写公司名称、业务简介和价值主张。</span>
            </div>
            <Link to='/sales/company'><Button type='primary'>前往填写</Button></Link>
          </div>
        ) : null}

        <SalesSection title='新建计划' description='国家可以输入多个；每个国家都会保留独立的研究证据和结果。'>
          <form className='sales-task-form' onSubmit={submit}>
            <div className='sales-form-grid'>
              <label className='sales-field sales-field--wide'>
                <span>任务名称 <em>必填</em></span>
                <Input value={draft.name} onChange={(name) => setDraft((value) => ({ ...value, name }))} placeholder='例如：日本生活杂货零售商开发' />
              </label>
              <label className='sales-field sales-field--wide'>
                <span>目标国家 <em>必填</em></span>
                <InputTag
                  value={draft.countries}
                  onChange={(countries) => setDraft((value) => ({ ...value, countries }))}
                  placeholder='输入后按回车或点击空白处，例如：日本'
                  saveOnBlur
                  allowClear
                />
              </label>
              <label className='sales-field'>
                <span>执行方式</span>
                <Select
                  value={draft.cadence}
                  onChange={(cadence) => setDraft((value) => ({ ...value, cadence: cadence as SalesTaskCadence }))}
                  options={[
                    { label: '仅执行一次', value: 'once' },
                    { label: '每天执行', value: 'daily' },
                  ]}
                />
              </label>
              <label className='sales-field'>
                <span>{draft.cadence === 'daily' ? '每天联系数量' : '本次公司数量'}</span>
                <InputNumber
                  min={1}
                  max={500}
                  value={draft.targetCount}
                  onChange={(targetCount) => setDraft((value) => ({ ...value, targetCount: Number(targetCount) || 1 }))}
                  suffix='家公司'
                />
              </label>
              <label className='sales-field'>
                <span>计划开始时间</span>
                <Input type='time' value={draft.scheduleTime} onChange={(scheduleTime) => setDraft((value) => ({ ...value, scheduleTime }))} />
              </label>
              <label className='sales-field sales-field--wide'>
                <span>筛选要求</span>
                <TextArea
                  value={draft.notes}
                  onChange={(notes) => setDraft((value) => ({ ...value, notes }))}
                  placeholder='例如：排除大型连锁集团；优先有进口商品和公开联系表单的公司。'
                  autoSize={{ minRows: 2, maxRows: 5 }}
                />
              </label>
            </div>
            <div className='sales-task-form__footer'>
              <span>
                {agentReady
                  ? '立即执行会打开 NomiFun Agent 会话；合格公司的联系表单将自动提交。'
                  : '保存计划不受影响；立即执行需要销售 Agent 设定和一个可用模型。'}
              </span>
              <div className='sales-task-form__actions'>
                <Button htmlType='submit' icon={<Plus size={15} />} disabled={!profileReady || executing}>
                  保存计划
                </Button>
                <Button
                  htmlType='button'
                  type='primary'
                  icon={<PlayOne size={15} />}
                  loading={executing}
                  disabled={!profileReady || presetLoading || !agentReady}
                  onClick={() => void executeNow()}
                >
                  立即执行
                </Button>
              </div>
            </div>
          </form>
        </SalesSection>

        <SalesSection title='已保存计划' description='可以移除不再需要的计划；已搜索公司和执行结果仍会保留。'>
          {visibleTasks.length > 0 ? (
            <div className='sales-task-list'>
              {visibleTasks.map((task) => {
                const companies = workspace.companies.filter((company) => company.taskId === task.id);
                const results = workspace.results.filter((result) => result.taskId === task.id);
                const submitted = results.filter((result) => result.outcome === 'submitted').length;
                const failed = results.filter((result) => result.outcome === 'failed').length;
                const expanded = expandedTaskIds.includes(task.id);
                const isExecutionTask = task.status !== 'draft' || Boolean(task.conversationId);

                return (
                  <article key={task.id} className='sales-task-item'>
                    <div className='sales-task-row'>
                      <div className='sales-task-row__icon'><Plan size={18} /></div>
                      <div className='sales-task-row__main'>
                        <div><strong>{task.name}</strong><SalesStatusPill status={task.status} /></div>
                        <p>{task.countries.join('、')} · {task.cadence === 'daily' ? `每天 ${task.targetCount} 家` : `共 ${task.targetCount} 家`} · {task.scheduleTime}</p>
                        {isExecutionTask ? (
                          <div className='sales-task-row__progress'>
                            <span>已搜索 {companies.length} / {task.targetCount}</span>
                            <span>已自动提交 {submitted}</span>
                            {failed > 0 ? <span className='is-warning'>需要处理 {failed}</span> : null}
                          </div>
                        ) : null}
                        {task.notes ? <small>{task.notes}</small> : null}
                      </div>
                      <div className='sales-task-row__meta'>
                        <time>{formatSalesDate(task.createdAt)}</time>
                        <div className='sales-task-row__actions'>
                          {isExecutionTask ? (
                            <Button
                              type='text'
                              size='small'
                              aria-expanded={expanded}
                              aria-controls={`sales-task-companies-${task.id}`}
                              icon={expanded ? <Up size={13} /> : <Down size={13} />}
                              onClick={() => toggleTaskCompanies(task.id)}
                            >
                              {companies.length > 0 ? `查看 ${companies.length} 家公司` : '查看搜索进度'}
                            </Button>
                          ) : null}
                          <Popconfirm
                            title={
                              isExecutionTask
                                ? '从计划列表移除？Agent 会继续执行，已有和后续结果仍保存在“结果记录”。'
                                : '删除这个计划？'
                            }
                            okText='移除计划'
                            cancelText='取消'
                            okButtonProps={{ status: 'danger' }}
                            onOk={() => deleteSavedTask(task.id, isExecutionTask)}
                          >
                            <Button type='text' status='danger' size='small' icon={<Delete size={13} />}>
                              删除计划
                            </Button>
                          </Popconfirm>
                        </div>
                      </div>
                    </div>

                    {isExecutionTask && expanded ? (
                      <div id={`sales-task-companies-${task.id}`} className='sales-task-company-panel'>
                        <div className='sales-task-company-panel__header'>
                          <div>
                            <strong>Agent 搜索到的公司</strong>
                            <span>{task.conversationId ? '执行期间每 8 秒自动同步' : '旧任务未关联 Agent'}</span>
                          </div>
                          <Link to={`/sales/approvals?task=${encodeURIComponent(task.id)}`}>
                            完整执行看板 <Right size={13} />
                          </Link>
                        </div>

                        {companies.length > 0 ? (
                          <div className='sales-task-company-list'>
                            {companies.map((company) => (
                              <div key={company.id} className='sales-task-company-row'>
                                <div>
                                  <strong>{company.name}</strong>
                                  <span>{company.country}{company.industry ? ` · ${company.industry}` : ''}</span>
                                </div>
                                <p>{company.fitSummary || 'Agent 正在分析这家公司。'}</p>
                                <div>
                                  <SalesStatusPill status={company.status} />
                                  <time>{formatSalesDate(company.updatedAt)}</time>
                                </div>
                              </div>
                            ))}
                          </div>
                        ) : (
                          <div className='sales-task-company-panel__empty'>
                            <strong>{task.conversationId ? '正在等待第一家公司' : '旧任务没有可同步的公司记录'}</strong>
                            <span>
                              {task.conversationId
                                ? 'Agent 生成第一条结构化记录后会自动显示在这里。'
                                : '请新建计划并点击“立即执行”，新任务会自动关联搜索进度。'}
                            </span>
                          </div>
                        )}
                      </div>
                    ) : null}
                  </article>
                );
              })}
            </div>
          ) : (
            <div className='sales-inline-empty'>
              <Plan size={20} />
              <div><strong>还没有计划</strong><span>使用上面的表单创建第一个销售任务。</span></div>
            </div>
          )}
        </SalesSection>
      </div>
    </div>
  );
};

export default SalesTasksPage;
