/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * CrawlPage — job list, create wizard, live progress, and failure replay.
 */
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Button,
  Empty,
  Form,
  Input,
  InputNumber,
  Message,
  Modal,
  Popconfirm,
  Progress,
  Select,
  Space,
  Switch,
  Table,
  Tag,
  Typography,
} from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type {
  ICrawlJob,
  ICrawlTask,
  ICreateCrawlJobParams,
  CrawlJobStatus,
} from '@/common/adapter/ipcBridge';
import type { CrawlJobId } from '@/common/types/ids';
import { useKnowledgeBaseOptions } from '@/renderer/hooks/knowledge/useKnowledgeBaseOptions';
import { useCrawlJobs } from '../useCrawlJobs';

const STATUS_COLOR: Record<CrawlJobStatus, string> = {
  draft: 'gray',
  running: 'arcoblue',
  paused: 'orange',
  done: 'green',
  failed: 'red',
  cancelled: 'gray',
};

/** Ratio of settled tasks, used for the per-job progress bar. */
export function completionPercent(job: ICrawlJob): number {
  const p = job.progress;
  const total = p.pending + p.in_progress + p.done + p.failed + p.skipped;
  if (total === 0) return 0;
  return Math.round(((p.done + p.failed + p.skipped) / total) * 100);
}

/** Seeds are entered one per line; blank lines are not URLs. */
export function parseSeeds(raw: string): string[] {
  return raw
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

const CrawlPage: React.FC = () => {
  const { t } = useTranslation();
  const { jobs, loading, error, createJob, startJob, cancelJob, deleteJob, retryFailed } =
    useCrawlJobs();
  const { options: kbOptions, loading: kbLoading } = useKnowledgeBaseOptions();
  const [creating, setCreating] = useState(false);
  const [tasksFor, setTasksFor] = useState<ICrawlJob | undefined>();
  const [tasks, setTasks] = useState<ICrawlTask[]>([]);
  const [form] = Form.useForm();

  const openTasks = async (job: ICrawlJob) => {
    setTasksFor(job);
    try {
      const rows = await ipcBridge.crawl.listTasks.invoke({ job_id: job.job_id, limit: 200 });
      setTasks(rows ?? []);
    } catch (e) {
      Message.error(e instanceof Error ? e.message : String(e));
    }
  };

  const submit = async () => {
    const values = await form.validate();
    const seeds = parseSeeds(String(values.seeds ?? ''));
    if (seeds.length === 0) {
      Message.error(t('crawl.error.noSeeds'));
      return;
    }
    const params: ICreateCrawlJobParams = {
      name: values.name,
      seeds,
      max_depth: values.max_depth,
      max_urls: values.max_urls,
      concurrency: values.concurrency,
      per_host_concurrency: values.per_host_concurrency,
      delay_ms: values.delay_ms,
      respect_robots: values.respect_robots,
      render_mode: values.render_mode,
      scope: { same_site: values.same_site },
      sink: {
        knowledge_base_id: values.knowledge_base_id || undefined,
        via_inbox: values.via_inbox,
      },
    };
    try {
      await createJob(params);
      setCreating(false);
      form.resetFields();
      Message.success(t('crawl.created'));
    } catch (e) {
      Message.error(e instanceof Error ? e.message : String(e));
    }
  };

  const act = async (fn: () => Promise<unknown>, okMessage?: string) => {
    try {
      await fn();
      if (okMessage) Message.success(okMessage);
    } catch (e) {
      Message.error(e instanceof Error ? e.message : String(e));
    }
  };

  const columns = useMemo(
    () => [
      {
        title: t('crawl.column.name'),
        dataIndex: 'name',
        render: (_: unknown, job: ICrawlJob) => (
          <Space direction='vertical' size={2}>
            <Typography.Text style={{ fontWeight: 500 }}>{job.name}</Typography.Text>
            <Typography.Text type='secondary' style={{ fontSize: 12 }}>
              {job.seeds[0]}
              {job.seeds.length > 1 ? ` +${job.seeds.length - 1}` : ''}
            </Typography.Text>
          </Space>
        ),
      },
      {
        title: t('crawl.column.status'),
        dataIndex: 'status',
        width: 110,
        render: (status: CrawlJobStatus) => (
          <Tag color={STATUS_COLOR[status]}>{t(`crawl.status.${status}`)}</Tag>
        ),
      },
      {
        title: t('crawl.column.progress'),
        width: 220,
        render: (_: unknown, job: ICrawlJob) => (
          <Space direction='vertical' size={2} style={{ width: '100%' }}>
            <Progress percent={completionPercent(job)} size='small' />
            <Typography.Text type='secondary' style={{ fontSize: 12 }}>
              {t('crawl.progressDetail', {
                done: job.progress.done,
                failed: job.progress.failed,
                pending: job.progress.pending + job.progress.in_progress,
              })}
            </Typography.Text>
          </Space>
        ),
      },
      {
        title: t('crawl.column.actions'),
        width: 300,
        render: (_: unknown, job: ICrawlJob) => (
          <Space>
            {job.status === 'running' ? (
              <Button size='mini' onClick={() => void act(() => cancelJob(job.job_id))}>
                {t('crawl.action.cancel')}
              </Button>
            ) : (
              <Button
                size='mini'
                type='primary'
                onClick={() => void act(() => startJob(job.job_id))}
              >
                {t('crawl.action.start')}
              </Button>
            )}
            <Button size='mini' onClick={() => void openTasks(job)}>
              {t('crawl.action.tasks')}
            </Button>
            {job.progress.failed > 0 && (
              <Button
                size='mini'
                onClick={() =>
                  void act(async () => {
                    const n = await retryFailed(job.job_id);
                    Message.success(t('crawl.requeued', { count: n }));
                  })
                }
              >
                {t('crawl.action.retryFailed')}
              </Button>
            )}
            <Popconfirm
              title={t('crawl.confirmDelete')}
              onOk={() => void act(() => deleteJob(job.job_id))}
            >
              <Button size='mini' status='danger'>
                {t('crawl.action.delete')}
              </Button>
            </Popconfirm>
          </Space>
        ),
      },
    ],
    [t, cancelJob, startJob, deleteJob, retryFailed]
  );

  const taskColumns = useMemo(
    () => [
      { title: t('crawl.column.url'), dataIndex: 'url', ellipsis: true },
      {
        title: t('crawl.column.status'),
        dataIndex: 'status',
        width: 110,
        render: (status: string) => <Tag>{t(`crawl.taskStatus.${status}`)}</Tag>,
      },
      { title: t('crawl.column.depth'), dataIndex: 'depth', width: 70 },
      { title: t('crawl.column.httpStatus'), dataIndex: 'http_status', width: 90 },
      { title: t('crawl.column.error'), dataIndex: 'error_detail', ellipsis: true },
    ],
    [t]
  );

  return (
    <div style={{ padding: 16 }}>
      <Space style={{ marginBottom: 16, justifyContent: 'space-between', width: '100%' }}>
        <Typography.Title heading={5} style={{ margin: 0 }}>
          {t('crawl.title')}
        </Typography.Title>
        <Button type='primary' onClick={() => setCreating(true)}>
          {t('crawl.action.create')}
        </Button>
      </Space>

      {error && (
        <Typography.Text type='error' style={{ display: 'block', marginBottom: 12 }}>
          {error}
        </Typography.Text>
      )}

      <Table
        rowKey='job_id'
        loading={loading}
        columns={columns}
        data={jobs}
        pagination={false}
        noDataElement={<Empty description={t('crawl.empty')} />}
      />

      <Modal
        title={t('crawl.action.create')}
        visible={creating}
        onOk={() => void submit()}
        onCancel={() => setCreating(false)}
        autoFocus={false}
      >
        <Form
          form={form}
          layout='vertical'
          initialValues={{
            max_depth: 3,
            max_urls: 10000,
            concurrency: 4,
            per_host_concurrency: 2,
            delay_ms: 500,
            respect_robots: true,
            same_site: true,
            via_inbox: true,
            render_mode: 'auto',
          }}
        >
          <Form.Item label={t('crawl.field.name')} field='name' rules={[{ required: true }]}>
            <Input placeholder={t('crawl.field.namePlaceholder')} />
          </Form.Item>
          <Form.Item label={t('crawl.field.seeds')} field='seeds' rules={[{ required: true }]}>
            <Input.TextArea rows={3} placeholder={'https://example.com/docs'} />
          </Form.Item>
          <Form.Item label={t('crawl.field.maxDepth')} field='max_depth'>
            <InputNumber min={0} max={20} />
          </Form.Item>
          <Form.Item label={t('crawl.field.maxUrls')} field='max_urls'>
            <InputNumber min={1} max={1000000} />
          </Form.Item>
          <Form.Item label={t('crawl.field.concurrency')} field='concurrency'>
            <InputNumber min={1} max={64} />
          </Form.Item>
          <Form.Item label={t('crawl.field.perHostConcurrency')} field='per_host_concurrency'>
            <InputNumber min={1} max={16} />
          </Form.Item>
          <Form.Item label={t('crawl.field.delayMs')} field='delay_ms'>
            <InputNumber min={0} max={600000} />
          </Form.Item>
          <Form.Item label={t('crawl.field.renderMode')} field='render_mode'>
            <Select
              options={[
                { label: t('crawl.renderMode.auto'), value: 'auto' },
                { label: t('crawl.renderMode.http'), value: 'http' },
                { label: t('crawl.renderMode.browser'), value: 'browser', disabled: true },
              ]}
            />
          </Form.Item>
          <Form.Item
            label={t('crawl.field.sameSite')}
            field='same_site'
            triggerPropName='checked'
            extra={t('crawl.field.sameSiteHint')}
          >
            <Switch />
          </Form.Item>
          <Form.Item
            label={t('crawl.field.respectRobots')}
            field='respect_robots'
            triggerPropName='checked'
            extra={t('crawl.field.respectRobotsHint')}
          >
            <Switch />
          </Form.Item>
          <Form.Item
            label={t('crawl.field.knowledgeBase')}
            field='knowledge_base_id'
            extra={t('crawl.field.knowledgeBaseHint')}
          >
            <Select
              placeholder={t('crawl.field.knowledgeBasePlaceholder')}
              loading={kbLoading}
              allowClear
              notFoundContent={<Empty description={t('crawl.field.knowledgeBaseEmpty')} />}
            >
              {kbOptions.map((option) => (
                <Select.Option key={option.value} value={option.value}>
                  {option.label}
                  <span style={{ color: 'var(--color-text-3)', marginLeft: 8 }}>
                    {option.rootPath}
                  </span>
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            label={t('crawl.field.viaInbox')}
            field='via_inbox'
            triggerPropName='checked'
            extra={t('crawl.field.viaInboxHint')}
          >
            <Switch />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title={tasksFor?.name}
        visible={!!tasksFor}
        onCancel={() => setTasksFor(undefined)}
        footer={null}
        style={{ width: 900 }}
      >
        <Table
          rowKey='task_id'
          columns={taskColumns}
          data={tasks}
          pagination={{ pageSize: 20 }}
          size='small'
        />
      </Modal>
    </div>
  );
};

export default CrawlPage;
