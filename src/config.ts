import type { ConfigGroup } from 'cortico/core/types.ts';
import type { Language } from 'cortico/core/language.ts';

export const SOUND_DEL_ID = 'sound-del';
export const SOUND_DEL_DEFAULTS: Readonly<SoundDelConfigSection> = {
  enabled: false,
  webUiUrl: 'http://127.0.0.1:17788',
  maxMuteSeconds: 300,
};

export interface SoundDelConfigSection {
  enabled: boolean;
  webUiUrl: string;
  maxMuteSeconds: number;
}

export function localOrigin(value: string): string {
  const url = new URL(value);
  if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)
    || url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw new Error('webUiUrl must be a loopback HTTP origin, for example http://127.0.0.1:17788');
  }
  if (url.hostname === 'localhost') url.hostname = '127.0.0.1';
  return url.origin;
}

export function validateConfig(config: SoundDelConfigSection): void {
  localOrigin(config.webUiUrl);
  if (!Number.isInteger(config.maxMuteSeconds) || config.maxMuteSeconds < 10 || config.maxMuteSeconds > 900) {
    throw new Error('maxMuteSeconds must be an integer between 10 and 900');
  }
}

export function soundDelConfigGroup(language: Language = 'zh'): ConfigGroup {
  const zh = language.startsWith('zh');
  return {
    id: 'world:sound-del',
    owner: 'world:sound-del',
    schema: {
      type: 'object',
      title: zh ? '收音静音' : 'Sound Del',
      description: zh
        ? '仅支持 Windows 按键收音。设备上的所有声音（包括 Coo）一起静音；始终监听模式不启用。'
        : 'Windows push-to-talk only. All playback on the device, including Coo, is muted. Always-listening mode is excluded.',
      properties: {
        'worlds.sound-del.maxMuteSeconds': {
          type: 'integer',
          title: zh ? '单次静音上限' : 'Maximum mute duration',
          minimum: 10,
          maximum: 900,
          'x-suffix': zh ? '秒' : 'seconds',
          'x-hot': false,
        },
      },
    },
  };
}
