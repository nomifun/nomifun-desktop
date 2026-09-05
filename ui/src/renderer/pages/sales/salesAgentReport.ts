import type { SalesCompany, SalesCompanyStatus, SalesResult, SalesTaskStatus, SalesWorkspace } from './salesWorkspace';

export const SALES_DASHBOARD_EVENT_PREFIX = 'SALES_DASHBOARD_EVENT ';

export type SalesDashboardEventStatus =
  | 'researching'
  | 'qualified'
  | 'not_qualified'
  | 'submitted'
  | 'failed';

export type SalesDashboardEvent = {
  eventId: string;
  companyName: string;
  website: string;
  country: string;
  industry: string;
  status: SalesDashboardEventStatus;
  fitSummary: string;
  evidenceUrl: string;
  formUrl: string;
  outreachMessage: string;
  restrictionSummary: string;
  detail: string;
  occurredAt: string;
};

export type SalesAgentSnapshot = {
  taskId: string;
  taskStatus: SalesTaskStatus;
  agentSummary: string;
  syncedAt: string;
  events: SalesDashboardEvent[];
};

const text = (value: unknown) => (typeof value === 'string' ? value.trim() : '');

const eventStatuses = new Set<SalesDashboardEventStatus>([
  'researching',
  'qualified',
  'not_qualified',
  'submitted',
  'failed',
]);

const stableEventId = (value: Record<string, unknown>, status: SalesDashboardEventStatus) => {
  const explicit = text(value.event_id);
  if (explicit) return explicit;
  const target = text(value.website) || text(value.company_name) || 'unknown-company';
  return `${target.toLowerCase()}:${status}`;
};

export const parseSalesDashboardEvent = (raw: unknown): SalesDashboardEvent | null => {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const status = text(value.status) as SalesDashboardEventStatus;
  const companyName = text(value.company_name);
  if (!companyName || !eventStatuses.has(status)) return null;

  return {
    eventId: stableEventId(value, status),
    companyName,
    website: text(value.website),
    country: text(value.country),
    industry: text(value.industry),
    status,
    fitSummary: text(value.fit_summary),
    evidenceUrl: text(value.evidence_url),
    formUrl: text(value.form_url),
    outreachMessage: text(value.outreach_message),
    restrictionSummary: text(value.restriction_summary),
    detail: text(value.detail),
    occurredAt: text(value.occurred_at),
  };
};

export const extractSalesDashboardEvents = (content: string): SalesDashboardEvent[] => {
  const events = new Map<string, SalesDashboardEvent>();
  for (const line of content.split(/\r?\n/)) {
    const marker = line.indexOf(SALES_DASHBOARD_EVENT_PREFIX);
    if (marker < 0) continue;
    const json = line.slice(marker + SALES_DASHBOARD_EVENT_PREFIX.length).trim();
    try {
      const event = parseSalesDashboardEvent(JSON.parse(json));
      if (event) events.set(event.eventId, event);
    } catch {
      // Streaming can expose a partial line. Ignore it until a later sync sees valid JSON.
    }
  }
  return [...events.values()];
};

const companyStatusForEvent = (status: SalesDashboardEventStatus): SalesCompanyStatus => {
  if (status === 'not_qualified') return 'skipped';
  return status;
};

const resultOutcomeForEvent = (status: SalesDashboardEventStatus): SalesResult['outcome'] | null => {
  if (status === 'submitted') return 'submitted';
  if (status === 'not_qualified') return 'skipped';
  if (status === 'failed') return 'failed';
  return null;
};

const companyKey = (taskId: string, event: SalesDashboardEvent) =>
  `${taskId}:${(event.website || event.companyName).trim().toLowerCase()}`;

export const mergeSalesAgentSnapshot = (
  workspace: SalesWorkspace,
  snapshot: SalesAgentSnapshot
): SalesWorkspace => {
  const task = workspace.tasks.find((item) => item.id === snapshot.taskId);
  if (!task) return workspace;

  const companyByKey = new Map(
    workspace.companies.map((company) => [companyKey(company.taskId, {
      eventId: '',
      companyName: company.name,
      website: company.website,
      country: company.country,
      industry: company.industry,
      status: 'researching',
      fitSummary: '',
      evidenceUrl: '',
      formUrl: '',
      outreachMessage: '',
      restrictionSummary: '',
      detail: '',
      occurredAt: '',
    }), company])
  );
  const resultEventIds = new Set(workspace.results.map((result) => result.eventId).filter(Boolean));
  const nextCompanies = [...workspace.companies];
  const newResults: SalesResult[] = [];

  for (const event of snapshot.events) {
    const key = companyKey(snapshot.taskId, event);
    const existing = companyByKey.get(key);
    const updatedAt = event.occurredAt || existing?.updatedAt || snapshot.syncedAt;
    const company: SalesCompany = {
      id: existing?.id ?? `sales-company:${key}`,
      taskId: snapshot.taskId,
      name: event.companyName,
      website: event.website || existing?.website || '',
      country: event.country || existing?.country || task.countries[0] || '—',
      industry: event.industry || existing?.industry || '',
      fitSummary: event.fitSummary || event.detail || existing?.fitSummary || '',
      evidenceUrl: event.evidenceUrl || existing?.evidenceUrl,
      contactFormUrl: event.formUrl || existing?.contactFormUrl,
      outreachMessage: event.outreachMessage || existing?.outreachMessage,
      restrictionSummary: event.restrictionSummary || existing?.restrictionSummary,
      lastEventId: event.eventId,
      status: companyStatusForEvent(event.status),
      updatedAt,
    };

    if (existing) {
      nextCompanies[nextCompanies.findIndex((item) => item.id === existing.id)] = company;
    } else {
      nextCompanies.unshift(company);
    }
    companyByKey.set(key, company);

    const outcome = resultOutcomeForEvent(event.status);
    if (outcome && !resultEventIds.has(event.eventId)) {
      newResults.push({
        id: `sales-result:${event.eventId}`,
        taskId: snapshot.taskId,
        eventId: event.eventId,
        companyId: company.id,
        companyName: company.name,
        country: company.country,
        outcome,
        detail: event.detail || (outcome === 'submitted' ? '联系表单已自动提交。' : 'Agent 已记录处理结果。'),
        formUrl: company.contactFormUrl,
        message: company.outreachMessage,
        completedAt: updatedAt,
      });
      resultEventIds.add(event.eventId);
    }
  }

  return {
    ...workspace,
    tasks: workspace.tasks.map((item) =>
      item.id === snapshot.taskId
        ? {
            ...item,
            status: snapshot.taskStatus,
            agentSummary: snapshot.agentSummary || item.agentSummary,
            agentUpdatedAt: snapshot.syncedAt,
          }
        : item
    ),
    companies: nextCompanies,
    results: [...newResults, ...workspace.results],
  };
};
