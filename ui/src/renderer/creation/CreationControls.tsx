import { useState } from 'react';
import { Message, Popover } from '@arco-design/web-react';
import { AddPicture, VideoTwo, Music, Down, ImageFiles, PageTemplate, MessageOne, Brain } from '@icon-park/react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import { useCreativeAssetPickerDialog } from '@/renderer/pages/creativeStudio/assets/useCreativeAssetPickerDialog';
import CreativeMediaPreview from '@/renderer/pages/creativeStudio/assets/components/CreativeMediaPreview';
import ComposerAttachmentTile from '@/renderer/components/chat/ComposerAttachmentTile';
import ImageLightbox from '@/renderer/components/media/ImageLightbox';
import CreativeAssetPreviewModal from '@/renderer/pages/creativeStudio/assets/page/CreativeAssetPreviewModal';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { useCreationComposer } from './CreationComposerContext';
import { useGenerationModel } from './useGenerationModel';
import type { CreationMode, CreationParameters } from './types';
import { inputsForMode } from './types';
import styles from './CreationControls.module.css';
import { creationCount, creationMaxCount, creationParameterPolicy, creationVideoSizeOptions, normalizeCreationParameters } from './parameterPolicy';
import ImageSizePicker from './parameters/ImageSizePicker';
import { imageGenerationAspectRatioValue, imageGenerationResolutionLabel } from './parameters/image';

const modeLabels = { image: '图片生成', video: '视频生成', music: '音乐生成' };
const modeIcons = { image: AddPicture, video: VideoTwo, music: Music };

export function CreationReferences({ startIndex = 0 }: { startIndex?: number }) {
  const creation = useCreationComposer();
  const [preview, setPreview] = useState<CreativeAsset | null>(null);
  const [opening, setOpening] = useState(false);
  if (!creation?.draft.references.length) return null;
  const { draft, update } = creation;
  const active = draft.mode ? inputsForMode(draft.mode, draft.references) : [];
  const open = async (id: string) => {
    setOpening(true);
    try {
      const asset = await creativeAssetClient.get(id);
      if (asset.deletedAt) throw new Error('此素材已删除');
      setPreview(asset);
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setOpening(false); }
  };
  return <>{draft.references.map((ref, index) => {
    const inactive = !active.some(input => input.asset_id === ref.asset_id);
    const label = ref.kind === 'image' ? '查看图片' : '查看文件';
    return <ComposerAttachmentTile key={ref.asset_id} title={ref.title} detail={ref.title + (inactive ? ' · 本轮不使用' : '')} ordinal={startIndex + index + 1} inactive={inactive} onRemove={() => update(current => ({ ...current, references: current.references.filter(item => item.asset_id !== ref.asset_id) }))}>
      <button type='button' className={styles.referencePreview} aria-label={label + '：' + ref.title} disabled={opening} onClick={() => void open(ref.asset_id)}>
        <CreativeMediaPreview kind={ref.kind} src={ref.kind === 'image' ? ref.url : undefined} alt='' />
      </button>
    </ComposerAttachmentTile>;
  })}
    {preview?.kind === 'image' ? <ImageLightbox key={preview.originalUrl} src={preview.originalUrl} title={preview.title} onClose={() => setPreview(null)} />
      : <CreativeAssetPreviewModal asset={preview} onClose={() => setPreview(null)} onDownload={asset => {
        const link = document.createElement('a'); link.href = asset.originalUrl; link.download = asset.title; link.click();
      }} />}
  </>;
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
  const picker = useCreativeAssetPickerDialog();
  const [busy, setBusy] = useState(false);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const params = draft.parameters[mode];
  const parameterPolicy = creationParameterPolicy(model.selected);
  const label = (key: CreationMode) => t(`creation.mode.${key}`, { defaultValue: modeLabels[key] });
  const setParam = (key: string, value: string | number | boolean) => update(current => ({ ...current, parameters: { ...current.parameters, [mode]: { ...current.parameters[mode], [key]: value } } }));
  const pickAssets = async () => {
    try {
      const ids = await picker.pick({ title: '资产库', acceptedKinds: mode === 'music' ? ['text'] : ['image', 'text'], initialSelectedIds: mode === 'music' ? [] : draft.references.filter(ref => ref.kind === 'image').map(ref => ref.asset_id) });
      if (!ids) return;
      setBusy(true);
      const assets = await Promise.all(ids.map(id => creativeAssetClient.get(id)));
      const text = assets.filter(asset => asset.kind === 'text').map(asset => asset.textContent || '').filter(Boolean).join('\n');
      if (text) onPromptChange([prompt, text].filter(Boolean).join('\n'));
      if (mode !== 'music') update(current => ({ ...current, references: [
        ...current.references.filter(ref => ref.kind !== 'image'),
        ...assets.filter(asset => asset.kind === 'image').map(asset => current.references.find(ref => ref.asset_id === asset.id) || ({ asset_id: asset.id, kind: asset.kind, role: 'reference' as const, title: asset.title, url: asset.thumbnailUrl || asset.originalUrl })),
      ] }));
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const modeMenu = <div className={styles.modeMenu}>
    <button type='button' className={styles.button} onClick={() => creation.exit()}><MessageOne size={15} />对话</button>
    {(['image', 'video', 'music'] as const).map(key => { const Icon = modeIcons[key]; return <button type='button' className={styles.button} key={key} onClick={() => creation.selectMode(key)}><Icon size={15} />{label(key)}</button>; })}
  </div>;
  if (!draft.mode) return <span className={styles.controls}>{(['image', 'video', 'music'] as const).map(key => { const Icon = modeIcons[key]; return <button type='button' data-creation-mode={key} aria-label={label(key)} className={styles.button} key={key} onClick={() => creation.selectMode(key)}><Icon size={15} /><span className='sendbox-responsive-label'>{label(key)}</span></button>; })}</span>;
  const Icon = modeIcons[mode];
  const sizes = model.sizePolicy.options.filter(option => !option.disabled);
  const selectedSize = sizes.find(option => (params.size !== undefined && option.requestSize === params.size) || (params.aspect !== undefined && option.value === params.aspect)) || sizes[0];
  const field = (name: string, key: string, values: Array<string | number>, fallback?: string | number) => <label className={styles.field}>{name}<select aria-label={name} value={String(params[key] ?? fallback ?? '')} onChange={event => setParam(key, typeof values[0] === 'number' ? Number(event.target.value) : event.target.value)}><option value=''>自动</option>{values.map(value => <option key={value} value={value}>{value}</option>)}</select></label>;
  const count = creationCount(mode, params.count, model.selected);
  const videoSizes = creationVideoSizeOptions(model.selected);
  const videoSize = videoSizes.find(option => option.value === params.size) || videoSizes[0];
  const videoSizeSummary = videoSize && videoSize.value !== 'auto'
    ? `${imageGenerationAspectRatioValue(videoSize)} · ${imageGenerationResolutionLabel(videoSize)}`
    : params.size || '自动';
  const summary = mode === 'image'
    ? [selectedSize?.value === 'auto' ? '自动' : selectedSize ? imageGenerationAspectRatioValue(selectedSize) : '比例', selectedSize && selectedSize.value !== 'auto' ? imageGenerationResolutionLabel(selectedSize) : null, `${count} 张`].filter(Boolean).join(' · ')
    : mode === 'video' ? `${videoSizeSummary} · ${params.seconds ? `${params.seconds}s` : '自动时长'} · ${count} 个`
    : params.instrumental === false ? '带歌词' : '纯音乐';
  const parameterPanel = <div className={styles.parameterPanel} data-testid='creation-parameter-panel'>
    {mode === 'image' && <ImageSizePicker options={sizes} value={selectedSize?.value || ''} disabled={!model.selected} onChange={size => {
      update(current => ({ ...current, parameters: { ...current.parameters, image: { ...current.parameters.image, aspect: size.value, size: size.requestSize || '', width: size.width, height: size.height } } }));
    }} />}
    {mode === 'video' && <>
      {videoSizes.length > 0 ? <ImageSizePicker options={videoSizes} value={videoSize?.value || 'auto'} disabled={!model.selected} onChange={size => {
        update(current => {
          const video: CreationParameters = { ...current.parameters.video, size: size.requestSize || '' };
          delete video.aspect;
          delete video.resolution;
          return { ...current, parameters: { ...current.parameters, video } };
        });
      }} /> : parameterPolicy.video.sizes.length > 0 && field('分辨率', 'size', parameterPolicy.video.sizes)}
      {parameterPolicy.video.seconds.length > 0 && field('时长（秒）', 'seconds', parameterPolicy.video.seconds)}
    </>}
    {mode !== 'music' && <fieldset className={styles.parameterGroup}><legend>生成数量</legend><div className={styles.quantityOptions}>
      {Array.from({ length: creationMaxCount(mode, model.selected) }, (_, i) => i + 1).map(value => <button type='button' key={value} aria-pressed={count === value} disabled={!model.selected} onClick={() => setParam('count', value)}>{value}</button>)}
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
      <button type='button' className={styles.button} disabled={busy} onClick={() => void pickAssets()}><ImageFiles size={15} />资产库</button>
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
    <span className={styles.chip}><Popover trigger='click' className={styles.modePopover} content={modeMenu}><button type='button' className={styles.button} aria-label={label(mode)}><Icon size={15} /><span className='sendbox-responsive-label'>{label(mode)}</span><Down size={12} className='sendbox-responsive-chevron' /></button></Popover></span>
    <Popover trigger='click' position='top' className={styles.parameterPopover} content={modelPanel} popupVisible={modelMenuOpen} onVisibleChange={setModelMenuOpen}><button type='button' className={`${styles.button} ${styles.model}`} aria-label='生成模型' aria-expanded={modelMenuOpen} title={model.selected ? `${model.selected.label} · ${model.selected.providerLabel}` : undefined}><Brain size={15} /><span className='sendbox-responsive-label'>{model.selected?.label || '选择模型'}</span><Down size={12} className='sendbox-responsive-chevron' /></button></Popover>
    <Popover trigger='click' position='top' className={styles.parameterPopover} style={{ maxWidth: 'none' }} content={parameterPanel}><button type='button' className={styles.button} aria-label={`生成参数：${summary}`}><span className={styles.summaryShape} aria-hidden='true' /><span className='sendbox-responsive-label'>{summary}</span><Down size={12} className='sendbox-responsive-chevron' /></button></Popover>
    {picker.dialog}
  </span>;
}
