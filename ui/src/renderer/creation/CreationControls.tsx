import { useState } from 'react';
import { Message, Popover } from '@arco-design/web-react';
import { AddPicture, VideoTwo, Music, Close, Down, ImageFiles, PageTemplate } from '@icon-park/react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import { useCreativeAssetPickerDialog } from '@/renderer/pages/creativeStudio/assets/useCreativeAssetPickerDialog';
import { useCreationComposer } from './CreationComposerContext';
import { useGenerationModel } from './useGenerationModel';
import type { CreationMode, CreationReference } from './types';
import { filesForCreation, inputsForMode } from './types';
import styles from './CreationControls.module.css';
import { creationParameterPolicy, creationVideoInputRoles, normalizeCreationParameters } from './parameterPolicy';
import ImageSizePicker from './parameters/ImageSizePicker';
import { imageGenerationAspectRatioValue, imageGenerationResolutionLabel } from './parameters/image';

const modeLabels = { image: '图片生成', video: '视频生成', music: '音乐生成' };
const modeIcons = { image: AddPicture, video: VideoTwo, music: Music };

export function CreationReferences() {
  const creation = useCreationComposer();
  const model = useGenerationModel(creation);
  if (!creation?.draft.references.length) return null;
  const { draft, update } = creation;
  const active = draft.mode ? inputsForMode(draft.mode, draft.references) : [];
  const roles = draft.mode === 'image' ? ['reference', 'mask'] : creationVideoInputRoles(model.selected);
  const roleLabel: Record<string, string> = { reference: '参考图', mask: '蒙版', first_frame: '首帧', last_frame: '尾帧' };
  return <div className={styles.references}>{draft.references.map(ref => {
    const input = active.find(input => input.asset_id === ref.asset_id);
    const unsupported = input && !roles.includes(input.role);
    return <div key={ref.asset_id} className={`${styles.reference} ${!input ? styles.inactive : ''}`}>
    {ref.url && ref.kind === 'image' && <img src={ref.url} alt='' />}<span>{ref.title}</span>
    {input && <select aria-label={`${ref.title}素材角色`} value={input.role} onChange={event => update(current => ({ ...current, references: current.references.map(item => item.asset_id === ref.asset_id ? { ...item, role: event.target.value as CreationReference['role'] } : item) }))}>
      {unsupported && <option value={input.role}>{roleLabel[input.role]}（需调整）</option>}{roles.map(role => <option key={role} value={role}>{roleLabel[role]}</option>)}
    </select>}{!input && <span>本轮不使用</span>}{unsupported && <span className={styles.error}>当前模型不支持，请调整角色</span>}
    <button type='button' aria-label={`移除${ref.title}`} onClick={() => update(current => ({ ...current, references: current.references.filter(item => item.asset_id !== ref.asset_id) }))}><Close size={12} /></button>
  </div>; })}</div>;
}

export default function CreationControls({ prompt, onPromptChange, files = [] }: { prompt: string; onPromptChange(value: string): void; files?: readonly string[] }) {
  const creation = useCreationComposer();
  return creation ? <Controls prompt={prompt} onPromptChange={onPromptChange} files={files} creation={creation} /> : null;
}

function Controls({ prompt, onPromptChange, files, creation }: { prompt: string; onPromptChange(value: string): void; files: readonly string[]; creation: NonNullable<ReturnType<typeof useCreationComposer>> }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { draft, update } = creation;
  const mode = draft.mode || draft.lastMode;
  const model = useGenerationModel(creation, files);
  const activeFiles = filesForCreation(draft.mode, files);
  const inactiveFiles = files.filter(file => !activeFiles.includes(file));
  const picker = useCreativeAssetPickerDialog();
  const [busy, setBusy] = useState(false);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const params = draft.parameters[mode];
  const parameterPolicy = creationParameterPolicy(model.selected);
  const label = (key: CreationMode) => t(`creation.mode.${key}`, { defaultValue: modeLabels[key] });
  const setParam = (key: string, value: string | number | boolean) => update(current => ({ ...current, parameters: { ...current.parameters, [mode]: { ...current.parameters[mode], [key]: value } } }));
  const pickAssets = async (text = false) => {
    try {
      const ids = await picker.pick({ acceptedKinds: text ? ['text'] : ['image'], initialSelectedIds: text ? [] : draft.references.map(ref => ref.asset_id) });
      if (!ids) return;
      setBusy(true);
      const assets = await Promise.all(ids.map(id => creativeAssetClient.get(id)));
      if (text) onPromptChange([prompt, ...assets.map(asset => asset.textContent || '')].filter(Boolean).join('\n'));
      else update(current => ({ ...current, references: assets.map(asset => current.references.find(ref => ref.asset_id === asset.id) || ({ asset_id: asset.id, kind: asset.kind, role: 'reference', title: asset.title, url: asset.thumbnailUrl || asset.originalUrl })) }));
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const modeMenu = <div className={styles.modeMenu}>{(['image', 'video', 'music'] as const).map(key => { const Icon = modeIcons[key]; return <button type='button' className={styles.button} key={key} onClick={() => creation.selectMode(key)}><Icon size={15} />{label(key)}</button>; })}</div>;
  if (!draft.mode) return <span className={styles.controls}>{(['image', 'video', 'music'] as const).map(key => { const Icon = modeIcons[key]; return <button type='button' data-creation-mode={key} className={styles.button} key={key} onClick={() => creation.selectMode(key)}><Icon size={15} />{label(key)}</button>; })}</span>;
  const Icon = modeIcons[mode];
  const sizes = model.sizePolicy.options.filter(option => !option.disabled);
  const selectedSize = sizes.find(option => (params.size !== undefined && option.requestSize === params.size) || (params.aspect !== undefined && option.value === params.aspect)) || sizes[0];
  const field = (name: string, key: string, values: Array<string | number>, fallback?: string | number) => <label className={styles.field}>{name}<select aria-label={name} value={String(params[key] ?? fallback ?? '')} onChange={event => setParam(key, typeof values[0] === 'number' ? Number(event.target.value) : event.target.value)}><option value=''>自动</option>{values.map(value => <option key={value} value={value}>{value}</option>)}</select></label>;
  const count = Math.min(mode === 'image' ? model.sizePolicy.maxCount : 8, Math.max(1, Number(params.count) || 1));
  const summary = mode === 'image'
    ? [selectedSize?.value === 'auto' ? '自动' : selectedSize ? imageGenerationAspectRatioValue(selectedSize) : '比例', selectedSize && selectedSize.value !== 'auto' ? imageGenerationResolutionLabel(selectedSize) : null, `${count} 张`].filter(Boolean).join(' · ')
    : mode === 'video' ? `${params.size || '自动'} · ${params.seconds ? `${params.seconds}s` : '自动时长'} · ${count} 个`
    : params.instrumental === false ? '带歌词' : '纯音乐';
  const parameterPanel = <div className={styles.parameterPanel} data-testid='creation-parameter-panel'>
    {mode === 'image' && <ImageSizePicker options={sizes} value={selectedSize?.value || ''} disabled={!model.selected} onChange={size => {
      update(current => ({ ...current, parameters: { ...current.parameters, image: { ...current.parameters.image, aspect: size.value, size: size.requestSize || '', width: size.width, height: size.height } } }));
    }} />}
    {mode === 'video' && <div className={styles.fields}>
      {parameterPolicy.video.sizes.length > 0 && field('比例 / 分辨率', 'size', parameterPolicy.video.sizes)}
      {parameterPolicy.video.seconds.length > 0 && field('时长（秒）', 'seconds', parameterPolicy.video.seconds)}
    </div>}
    {mode !== 'music' && <fieldset className={styles.parameterGroup}><legend>生成数量</legend><div className={styles.quantityOptions}>
      {Array.from({ length: mode === 'image' ? model.sizePolicy.maxCount : 8 }, (_, i) => i + 1).map(value => <button type='button' key={value} aria-pressed={count === value} disabled={!model.selected} onClick={() => setParam('count', value)}>{value}</button>)}
    </div></fieldset>}
    {mode === 'image' && parameterPolicy.qualities.length > 0 && field('质量', 'quality', parameterPolicy.qualities)}
    {mode === 'music' && <>
      <fieldset className={styles.parameterGroup}><legend>音乐类型</legend><div className={styles.quantityOptions}>
        <button type='button' aria-pressed={params.instrumental !== false} onClick={() => setParam('instrumental', true)}>纯音乐</button>
        <button type='button' aria-pressed={params.instrumental === false} onClick={() => setParam('instrumental', false)}>带歌词</button>
      </div></fieldset>
      {params.instrumental === false && <label className={styles.field}>歌词<textarea rows={5} aria-label='歌词' value={String(params.lyrics || '')} onChange={event => setParam('lyrics', event.target.value)} /></label>}
    </>}
    <div className={styles.panelActions}>
      {mode !== 'music' && <button type='button' className={styles.button} disabled={busy} onClick={() => void pickAssets()}><ImageFiles size={15} />我的素材</button>}
      <button type='button' className={styles.button} disabled={busy} onClick={() => void pickAssets(true)}>引用文本素材</button>
      <button type='button' className={styles.button} onClick={() => navigate('/asset-library/templates')}><PageTemplate size={15} />模板工作台</button>
    </div>
  </div>;
  const modelPanel = <div className={styles.modelPanel} data-testid='creation-model-panel'>
    {model.isLoading && <span className={styles.notice} role='status'>正在加载模型…</span>}
    {model.error && <span className={styles.notice} role='alert'>模型目录加载失败，请重试。</span>}
    {!model.isLoading && !model.error && model.options.length === 0 && <span className={styles.notice}>暂无可用的{label(mode)}模型</span>}
    {draft.models[mode] && !model.selected && !model.isLoading && <span className={styles.notice}>原生成模型不可用于当前任务，请重新选择。</span>}
    {model.options.map(option => <button type='button' className={styles.modelOption} key={`${option.providerId}/${option.model}`} aria-pressed={model.selected === option} onClick={() => {
      update(current => ({ ...current, models: { ...current.models, [mode]: { providerId: option.providerId, model: option.model } }, parameters: { ...current.parameters, [mode]: normalizeCreationParameters(mode, current.parameters[mode], option) } }));
      setModelMenuOpen(false);
    }}><span>{option.label}</span><span className={styles.notice}>{option.providerLabel}</span></button>)}
    <div className={styles.panelActions}>
      {model.error && <button type='button' className={styles.button} onClick={() => void model.refresh()}>重试模型目录</button>}
      <button type='button' className={styles.button} onClick={() => { setModelMenuOpen(false); navigate('/models'); }}>配置生成模型</button>
    </div>
  </div>;
  return <span className={styles.controls}>
    <span className={styles.chip}><Popover trigger='click' className={styles.modePopover} content={modeMenu}><button type='button' className={styles.button}><Icon size={15} />{label(mode)}<Down size={12} /></button></Popover></span>
    <Popover trigger='click' position='top' className={styles.parameterPopover} content={modelPanel} popupVisible={modelMenuOpen} onVisibleChange={setModelMenuOpen}><button type='button' className={`${styles.button} ${styles.model}`} aria-label='生成模型' aria-expanded={modelMenuOpen} title={model.selected ? `${model.selected.label} · ${model.selected.providerLabel}` : undefined}><span>{model.selected?.label || '选择模型'}</span><Down size={12} /></button></Popover>
    <Popover trigger='click' position='top' className={styles.parameterPopover} style={{ maxWidth: 'none' }} content={parameterPanel}><button type='button' className={styles.button} aria-label={`生成参数：${summary}`}><span className={styles.summaryShape} aria-hidden='true' />{summary}<Down size={12} /></button></Popover>
    {inactiveFiles.length > 0 && <span className={styles.notice} title={inactiveFiles.join('\n')}>{inactiveFiles.length} 个附件本轮不使用，附件已保留</span>}
    {picker.dialog}
  </span>;
}
