import { createContentSiderChannel } from '@/renderer/components/layout/ContentSider/createContentSiderChannel';

export const workbenchSiderChannels = {
  image: createContentSiderChannel('image-workbench', false),
  video: createContentSiderChannel('video-workbench', false),
};
