import type { ToolDef, World, WorldHost, WorldWebUiDecl, ShutdownExternalCheck } from 'cortico/core/types.ts';
import type { Language } from 'cortico/core/language.ts';
import type { WorldContext } from 'cortico/world.ts';
import { SOUND_DEL_ID, soundDelConfigGroup, validateConfig, type SoundDelConfigSection } from './config.js';
import { HelperClient, defaultDataDir } from './helper-client.js';
import { VoiceBridge } from './voice-bridge.js';

export class SoundDelWorld implements World {
  readonly id = SOUND_DEL_ID;
  private bridge: VoiceBridge | null = null;
  private helper: HelperClient | null = null;
  private restored = true;
  private log: WorldHost['log'] | null = null;
  private reportedError: string | null = null;

  constructor(private readonly context: WorldContext<SoundDelConfigSection>) {}

  envPromptVars(): null { return null; }
  tools(): ToolDef[] { return []; }

  async start(host: WorldHost): Promise<void> {
    if (this.helper) return;
    if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Sound Del requires Windows x64');
    validateConfig(this.context.cfg);
    this.log = host.log;
    this.reportedError = null;
    this.restored = false;
    this.bridge = new VoiceBridge({
      origin: this.context.cfg.webUiUrl,
      password: () => this.context.secret('CORTICO_SOUND_DEL_WEB_PASSWORD') || this.context.secret('CORTICO_WEB_PASSWORD'),
      changed: () => { this.helper?.changed(); this.reportFailure(); },
    });
    this.helper = new HelperClient({
      dataDir: defaultDataDir(),
      maxMuteSeconds: this.context.cfg.maxMuteSeconds,
      desired: () => this.context.cfg.enabled && (this.bridge?.policy.active(performance.now()) ?? false),
      hotkey: () => this.context.cfg.enabled ? this.bridge?.policy.directHotkey(performance.now()) ?? null : null,
      changed: () => this.reportFailure(),
    });
    try {
      await this.helper.start();
      this.bridge.start();
      this.log.info('Sound Del started; direct shortcut detection follows the local voice configuration');
    } catch (error) {
      await this.stop();
      throw error;
    }
  }

  private reportFailure(): void {
    const error = this.helper?.error ?? this.helper?.status.suspended
      ?? (this.bridge?.state === 'error' ? this.bridge.detail : null);
    if (error && error !== this.reportedError) this.log?.warn(`Sound Del: ${error}`);
    this.reportedError = error ?? null;
  }

  async stop(): Promise<void> {
    this.bridge?.stop();
    this.bridge = null;
    if (this.helper) this.restored = await this.helper.stop();
    this.helper = null;
    if (!this.restored) this.log?.warn('Sound Del restoration is not verified; use npm run recover after disabling this World');
  }

  webUi(language: Language = 'zh'): WorldWebUiDecl {
    const zh = language.startsWith('zh'), helper = this.helper, bridge = this.bridge;
    const failure = helper?.error ?? helper?.status.suspended;
    const connected = bridge?.policy.connected && bridge.policy.snapshot !== null;
    return {
      label: zh ? '收音静音' : 'Sound Del',
      config: [soundDelConfigGroup(language)],
      lamps: [{
        label: zh ? '收音静音' : 'Sound Del',
        state: failure || bridge?.state === 'error' ? 'error' : !helper ? 'offline' : !helper.ready || !connected ? 'loading' : 'online',
        hint: helper?.error ? (zh ? `静音辅助进程已停用，请重启此 World。${failure}` : `Mute helper stopped; restart this World. ${failure}`)
          : failure ??
          (helper?.status.directHotkey ? (zh ? '快捷键直接触发静音' : 'Shortcut directly triggers mute')
          : bridge?.detail ?? (zh ? '未启用' : 'Disabled')),
      }],
      badges: [
        { label: zh ? '已接管设备' : 'Owned devices', value: helper?.status.owned ?? 0, tone: helper?.status.active ? 'on' : 'off' },
        { label: zh ? '待恢复' : 'Pending restore', value: helper?.status.recoveryUncertain ? (zh ? '待确认' : 'Unknown') : helper?.status.pendingRestore ?? 0, tone: 'plain' },
      ],
    };
  }

  shutdownVerification(language: Language): readonly ShutdownExternalCheck[] {
    const zh = language.startsWith('zh');
    return [{
      key: 'sound-del-output',
      label: zh ? '播放设备静音恢复' : 'Playback mute restoration',
      status: this.restored ? 'verified-ended' : 'unknown',
      detail: this.restored
        ? (zh ? '辅助进程已退出，未留下待恢复设备。' : 'The helper exited without pending devices.')
        : (zh ? '恢复尚未确认；请检查恢复记录和设备状态。' : 'Restoration is not verified; check the recovery journal and device state.'),
      manualAction: zh ? '停用此 World，然后在插件目录运行 npm run recover。' : 'Disable this World, then run npm run recover in the extension directory.',
    }];
  }
}
