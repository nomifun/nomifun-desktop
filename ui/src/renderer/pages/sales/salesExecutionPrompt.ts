import type { NewSalesTask, SalesCompanyProfile } from './salesWorkspace';

export const SALES_OPERATOR_SKILL_NAME = 'sales-contact-operator';
export const SALES_OPERATOR_PRESET_NAME = '本地销售联络助手';

export const buildSalesExecutionPrompt = (
  profile: SalesCompanyProfile,
  task: NewSalesTask,
  taskId?: string
) => `请立即执行下面的跨境 B2B 销售联络任务。

必须加载并严格遵守 ${SALES_OPERATOR_SKILL_NAME} Skill，包括官网证据核实、联系限制检查、表单字段处理、自动提交边界和 Dashboard 事件格式。对于证据充分、符合筛选要求、且官网没有明确禁止销售联络的公司，填写并自动提交官方联系表单，不需要再次询问我。不得在证据不足、用途不符、网站禁止销售联络或遇到登录、验证码、访问限制时提交。

我的公司：${profile.companyName.trim()}
公司网站：${profile.website.trim() || '未填写'}
业务简介：${profile.businessSummary.trim()}
价值主张：${profile.valueProposition.trim()}
理想客户：${profile.targetCustomer.trim() || '未额外指定'}
默认联系人：${profile.senderName.trim() || '未填写'}
默认联系邮箱：${profile.senderEmail.trim() || '未填写'}

任务名称：${task.name.trim()}
任务 ID：${taskId || '未提供'}
目标国家：${task.countries.map((country) => country.trim()).filter(Boolean).join('、')}
目标公司数量：${Math.max(1, Math.round(task.targetCount))}
执行方式：${task.cadence === 'daily' ? '每日任务；本次先执行一轮' : '仅执行一次'}
筛选要求：${task.notes.trim() || '无额外要求'}

现在开始搜索、核实并逐家公司处理。每家公司最多提交一次；无论提交、跳过还是失败，都按 Skill 要求输出一条 SALES_DASHBOARD_EVENT 单行 JSON 记录，供执行看板同步进度和联络内容。`;
