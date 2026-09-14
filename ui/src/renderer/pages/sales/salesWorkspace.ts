export type SalesTaskCadence = 'once' | 'daily';
export type SalesTaskStatus = 'draft' | 'queued' | 'running' | 'paused' | 'completed' | 'failed';
export type SalesCompanyStatus =
  | 'sourcing'
  | 'researching'
  | 'qualified'
  | 'skipped'
  | 'form_ready'
  | 'submitted'
  | 'failed';

export type SalesCompanyProfile = {
  companyName: string;
  website: string;
  businessSummary: string;
  valueProposition: string;
  targetCustomer: string;
  senderName: string;
  senderEmail: string;
};

export type SalesTask = {
  id: string;
  name: string;
  cadence: SalesTaskCadence;
  targetCount: number;
  countries: string[];
  scheduleTime: string;
  notes: string;
  status: SalesTaskStatus;
  createdAt: string;
  conversationId?: string;
  agentSummary?: string;
  agentUpdatedAt?: string;
  archivedAt?: string;
};

export type SalesCompany = {
  id: string;
  taskId: string;
  name: string;
  website: string;
  country: string;
  industry: string;
  fitSummary: string;
  evidenceUrl?: string;
  contactFormUrl?: string;
  outreachMessage?: string;
  restrictionSummary?: string;
  lastEventId?: string;
  status: SalesCompanyStatus;
  updatedAt: string;
};

export type SalesResult = {
  id: string;
  taskId?: string;
  eventId?: string;
  companyId: string;
  companyName: string;
  country: string;
  outcome: 'submitted' | 'skipped' | 'failed';
  detail: string;
  formUrl?: string;
  message?: string;
  completedAt: string;
};

export type SalesWorkspace = {
  version: 1;
  companyProfile: SalesCompanyProfile;
  tasks: SalesTask[];
  companies: SalesCompany[];
  results: SalesResult[];
};

export type NewSalesTask = Pick<
  SalesTask,
  'name' | 'cadence' | 'targetCount' | 'countries' | 'scheduleTime' | 'notes'
>;

export const SALES_WORKSPACE_STORAGE_KEY = 'nomifun-sales-workspace-v1';

export const EMPTY_COMPANY_PROFILE: SalesCompanyProfile = {
  companyName: '',
  website: '',
  businessSummary: '',
  valueProposition: '',
  targetCustomer: '',
  senderName: '',
  senderEmail: '',
};

export const createEmptySalesWorkspace = (): SalesWorkspace => ({
  version: 1,
  companyProfile: { ...EMPTY_COMPANY_PROFILE },
  tasks: [],
  companies: [],
  results: [],
});

const text = (value: unknown) => (typeof value === 'string' ? value : '');
const stringList = (value: unknown) =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];

const parseTask = (value: unknown): SalesTask | null => {
  if (!value || typeof value !== 'object') return null;
  const task = value as Partial<SalesTask>;
  if (!text(task.id) || !text(task.name)) return null;
  const legacyExternalJobId = text((value as { kylonJobId?: unknown }).kylonJobId);
  const parsedStatus = ['draft', 'queued', 'running', 'paused', 'completed', 'failed'].includes(text(task.status))
    ? (task.status as SalesTaskStatus)
    : 'draft';
  const status = legacyExternalJobId && !text(task.conversationId) && parsedStatus === 'running'
    ? 'paused'
    : parsedStatus;
  return {
    id: text(task.id),
    name: text(task.name),
    cadence: task.cadence === 'daily' ? 'daily' : 'once',
    targetCount: Number.isFinite(task.targetCount) ? Math.max(1, Number(task.targetCount)) : 1,
    countries: stringList(task.countries),
    scheduleTime: text(task.scheduleTime) || '09:00',
    notes: text(task.notes),
    status,
    createdAt: text(task.createdAt) || new Date(0).toISOString(),
    conversationId: text(task.conversationId) || undefined,
    agentSummary: legacyExternalJobId && !text(task.conversationId)
      ? '旧的外部 Agent 执行已停止；如需继续，请新建任务并交给 NomiFun Agent。'
      : text(task.agentSummary) || undefined,
    agentUpdatedAt: text(task.agentUpdatedAt) || undefined,
    archivedAt: text(task.archivedAt) || undefined,
  };
};

export const archiveSalesTask = (
  workspace: SalesWorkspace,
  taskId: string,
  archivedAt: string
): SalesWorkspace => ({
  ...workspace,
  tasks: workspace.tasks.map((task) => (task.id === taskId ? { ...task, archivedAt } : task)),
});

/**
 * Parse browser-persisted product-shell state defensively. Execution results
 * will move to the backend later; malformed local data must never prevent the
 * customer workspace from opening in the meantime.
 */
export const parseSalesWorkspace = (raw: string | null): SalesWorkspace => {
  if (!raw) return createEmptySalesWorkspace();
  try {
    const value = JSON.parse(raw) as Partial<SalesWorkspace>;
    if (!value || typeof value !== 'object' || value.version !== 1) return createEmptySalesWorkspace();
    const profile = value.companyProfile ?? EMPTY_COMPANY_PROFILE;
    return {
      version: 1,
      companyProfile: {
        companyName: text(profile.companyName),
        website: text(profile.website),
        businessSummary: text(profile.businessSummary),
        valueProposition: text(profile.valueProposition),
        targetCustomer: text(profile.targetCustomer),
        senderName: text(profile.senderName),
        senderEmail: text(profile.senderEmail),
      },
      tasks: Array.isArray(value.tasks) ? value.tasks.map(parseTask).filter((task): task is SalesTask => Boolean(task)) : [],
      companies: Array.isArray(value.companies) ? value.companies : [],
      results: Array.isArray(value.results) ? value.results : [],
    };
  } catch {
    return createEmptySalesWorkspace();
  }
};

export const buildSalesTask = (input: NewSalesTask, id: string, createdAt: string): SalesTask => ({
  id,
  name: input.name.trim(),
  cadence: input.cadence,
  targetCount: Math.max(1, Math.round(input.targetCount)),
  countries: [...new Set(input.countries.map((country) => country.trim()).filter(Boolean))],
  scheduleTime: input.scheduleTime || '09:00',
  notes: input.notes.trim(),
  status: 'draft',
  createdAt,
});

export const isCompanyProfileReady = (profile: SalesCompanyProfile) =>
  Boolean(profile.companyName.trim() && profile.businessSummary.trim() && profile.valueProposition.trim());
