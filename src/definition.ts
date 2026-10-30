import type { WorldDefinition } from 'cortico/world.ts';
import { SOUND_DEL_DEFAULTS, SOUND_DEL_ID, validateConfig, type SoundDelConfigSection } from './config.js';
import { SoundDelWorld } from './world.js';

export const SOUND_DEL: WorldDefinition<SoundDelConfigSection> = {
  id: SOUND_DEL_ID,
  label: 'Sound Del',
  defaults: () => ({ ...SOUND_DEL_DEFAULTS }),
  preflight: (context) => {
    if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Sound Del requires Windows x64');
    validateConfig(context.cfg);
  },
  create: (context) => new SoundDelWorld(context),
  describe: { tools: [], events: [] },
};
