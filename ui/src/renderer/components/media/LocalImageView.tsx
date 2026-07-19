import { ipcBridge } from '@/common';
import { resolveImageSource } from '@/common/utils/localPath';
import { LoadingTwo } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { createContext } from '@renderer/utils/ui/createContext';
import { iconColors } from '@/renderer/styles/colors';

const [useLocalImage, LocalImageProvider, useUpdateLocalImage] = createContext({ root: '' });

type ImageLoadState = {
  key: string;
  status: 'loading' | 'ready' | 'error';
  url?: string;
};

const LocalImageView: React.FC<{
  src: string;
  alt: string;
  className?: string;
}> & {
  Provider: typeof LocalImageProvider;
  useUpdateLocalImage: typeof useUpdateLocalImage;
} = ({ src, alt, className }) => {
  const { root } = useLocalImage();
  const resolved = useMemo(() => resolveImageSource(src, root), [src, root]);
  const sourceKey =
    resolved.kind === 'direct'
      ? `direct:${resolved.url}`
      : `local:${resolved.workspace ?? ''}\0${resolved.path}`;
  const [loadState, setLoadState] = useState<ImageLoadState>({ key: '', status: 'loading' });

  useEffect(() => {
    if (resolved.kind === 'direct') {
      setLoadState({ key: sourceKey, status: 'ready', url: resolved.url });
      return;
    }

    // Clear the previous image immediately. In addition to the cancellation
    // guard below, sourceKey prevents one render of stale content before this
    // effect runs when src/root changes.
    setLoadState({ key: sourceKey, status: 'loading' });
    if (!resolved.path) {
      setLoadState({ key: sourceKey, status: 'error' });
      return;
    }

    let cancelled = false;
    ipcBridge.fs.getImageBase64
      .invoke({ path: resolved.path, workspace: resolved.workspace })
      .then((base64) => {
        if (cancelled) return;
        if (!base64) throw new Error('Image file returned no data');
        setLoadState({ key: sourceKey, status: 'ready', url: base64 });
      })
      .catch((error) => {
        if (cancelled) return;
        console.error('[LocalImageView] Failed to load image:', {
          path: resolved.path,
          error,
        });
        setLoadState({ key: sourceKey, status: 'error' });
      });

    return () => {
      cancelled = true;
    };
  }, [resolved, sourceKey]);

  const currentState: ImageLoadState =
    loadState.key === sourceKey
      ? loadState
      : resolved.kind === 'direct'
        ? { key: sourceKey, status: 'ready', url: resolved.url }
        : { key: sourceKey, status: 'loading' };

  if (currentState.status === 'loading')
    return (
      <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
        <LoadingTwo
          className='loading'
          style={{ display: 'flex' }}
          theme='outline'
          size='14'
          fill={iconColors.primary}
          strokeWidth={2}
        />
        <span>{alt}</span>
      </span>
    );

  if (currentState.status === 'error' || !currentState.url) {
    return (
      <span role='status' aria-label={`Image unavailable: ${alt || src}`} className={className}>
        Image unavailable: {alt || src}
      </span>
    );
  }

  return (
    <img
      src={currentState.url}
      alt={alt}
      className={className}
      onError={() => setLoadState({ key: sourceKey, status: 'error' })}
    />
  );
};

LocalImageView.Provider = LocalImageProvider;
LocalImageView.useUpdateLocalImage = useUpdateLocalImage;

export default LocalImageView;
