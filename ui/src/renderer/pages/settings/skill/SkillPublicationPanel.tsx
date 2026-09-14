import { httpPost } from '@/common/adapter/httpBridge';
import type { SkillInfo } from '@/common/types/skill';
import { Alert, Button, Select } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

type Selection = { name: string; source: 'custom' | 'builtin' | 'builtin_auto' };
type Project = { project_id: string; project_revision: number; display_name: string };
type Preview = {
  selection: Selection;
  source_digest: string;
  artifact_digest: string;
  package_id: string;
  package_version: string;
  skill_id: string;
  files: Array<{ path: string; bytes: number; sha256: string }>;
  expected_library_revision: number;
  projects: Project[];
};
type PublishRequest = {
  selection: Selection;
  expected_source_digest: string;
  expected_artifact_digest: string;
  expected_library_revision: number;
  target_project_id?: string;
  expected_project_revision?: number;
};
const publication = {
  preview: httpPost<Preview, Selection>('/api/skills/frozen/preview'),
  publish: httpPost<{ project_id: string }, PublishRequest>('/api/skills/frozen/publish'),
};

/** Explicit candidate publication, separate from source editing or Agent selection. */
export default function SkillPublicationPanel({ skill, isAutoInjected }: { skill: SkillInfo; isAutoInjected: boolean }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [preview, setPreview] = useState<Preview | null>(null);
  const [target, setTarget] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  const [published, setPublished] = useState<string>();
  const sequence = useRef(0);
  const pending = useRef(false);
  useEffect(() => () => { sequence.current += 1; }, []);

  const inspect = async () => {
    if (pending.current) return;
    pending.current = true;
    const request = ++sequence.current;
    setBusy(true); setFailure(undefined); setPreview(null); setPublished(undefined);
    try {
      const value = await publication.preview.invoke({ name: skill.name, source: isAutoInjected ? 'builtin_auto' : skill.source });
      if (sequence.current !== request) return;
      setPreview(value);
      setTarget(value.projects.length === 1 ? value.projects[0].project_id : undefined);
    } catch (error) {
      if (sequence.current === request) setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      pending.current = false;
      if (sequence.current === request) setBusy(false);
    }
  };

  const publish = async () => {
    if (!preview || pending.current || (preview.projects.length > 0 && !target)) return;
    pending.current = true;
    const request = ++sequence.current;
    const project = preview.projects.find((item) => item.project_id === target);
    setBusy(true); setFailure(undefined);
    try {
      const result = await publication.publish.invoke({
        selection: preview.selection,
        expected_source_digest: preview.source_digest,
        expected_artifact_digest: preview.artifact_digest,
        expected_library_revision: preview.expected_library_revision,
        ...(project ? { target_project_id: project.project_id, expected_project_revision: project.project_revision } : {}),
      });
      if (sequence.current !== request) return;
      setPublished(result.project_id); setPreview(null);
    } catch (error) {
      if (sequence.current === request) {
        // No automatic retry: a disconnected response can conceal a committed candidate.
        setPreview(null);
        setFailure(error instanceof Error ? error.message : String(error));
      }
    } finally {
      pending.current = false;
      if (sequence.current === request) setBusy(false);
    }
  };

  return <section className='mt-12px shrink-0 rounded-12px border border-solid border-[var(--color-border-2)] p-12px' data-testid='skill-publication'>
    <div className='font-600'>{t('settings.skillsHub.freezeTitle', { defaultValue: 'Publish an immutable Agent Skill' })}</div>
    <p className='my-8px text-12px text-t-secondary'>{t('settings.skillsHub.freezeHelp', { defaultValue: 'Preview and freeze the complete Skill into a Plugin candidate. Review/apply and enable it in Plugins, then select it in the Agent workbench and publish a new revision. Existing Sessions are unchanged. Plugin Package v1 requires the managed Node runtime to enable; this is not an engine installation.' })}</p>
    <Button size='small' loading={busy} disabled={busy} onClick={() => void inspect()}>{t('settings.skillsHub.freezePreview', { defaultValue: 'Preview frozen files' })}</Button>
    {failure && <Alert className='mt-8px' type='error' content={failure} />}
    {preview && <div className='mt-8px flex flex-col gap-8px'>
      <code className='break-all text-11px'>{preview.artifact_digest}</code>
      <div className='max-h-140px overflow-auto text-12px'>{preview.files.map((file) => <div key={file.path} className='break-all' title={`SHA-256: ${file.sha256}`}>{file.path} · {file.bytes} B</div>)}</div>
      {preview.projects.length > 0 && <Select value={target} onChange={setTarget} disabled={busy}
        placeholder={t('settings.skillsHub.freezeTarget', { defaultValue: 'Choose the existing Skill project to update' })}
        options={preview.projects.map((project) => ({ value: project.project_id, label: `${project.display_name} · ${project.project_id}` }))} />}
      <Alert type='warning' content={t('settings.skillsHub.freezeConfirm', { defaultValue: 'This creates or replaces the selected project’s ready candidate only. No activation, Agent change, capability grant or script execution occurs. Source changes require another preview.' })} />
      <Button type='primary' size='small' loading={busy} disabled={busy || (preview.projects.length > 0 && !target)} onClick={() => void publish()}>{t('settings.skillsHub.freezePublish', { defaultValue: 'Confirm and create candidate' })}</Button>
    </div>}
    {published && <div className='mt-8px'>
      <Alert type='success' content={t('settings.skillsHub.freezeReady', { defaultValue: 'Frozen candidate created. Review it in Plugins before selecting it for an Agent.' })} />
      <code className='block break-all text-11px'>{published}</code>
      <Button className='mt-8px' size='small' onClick={() => navigate('/plugins?tab=workshop')}>{t('settings.skillsHub.freezeOpen', { defaultValue: 'Open Plugins workshop' })}</Button>
    </div>}
  </section>;
}
