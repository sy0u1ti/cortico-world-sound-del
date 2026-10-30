# 收音静音 / Sound Del

按下 Coopanion 原来的收音快捷键时静音播放设备，收音结束后恢复。

**不修改主程序，不更换快捷键，不模拟或吞掉按键，不安装虚拟声卡，不接管麦克风。**

[English](README.md) · [下载](https://github.com/sy0u1ti/cortico-world-sound-del/releases) · [反馈问题](https://github.com/sy0u1ti/cortico-world-sound-del/issues)

## 使用条件

- Windows x64，Cortico World API 6。
- Coopanion 内置的 `desktop-pet` 语音模块，已启用按键收音。
- 单独运行开发命令时，需要 Node.js 22.12 或以上。

这是第三方扩展，并非 Pal AI Lab 官方产品。插件通过已有的本机语音面板接口同步快捷键和收音状态，因此声明 `requires-extension` 标签；它不是独立录音程序。

## 安装与更新

npm 发布并完成市场同步后，在 Coopanion 的 **World 扩展 → 扩展市场** 搜索：

```text
cortico-world-sound-del
```

中文显示名称为「收音静音」。安装后按提示重启，再启用这个 World。

也可以从 [Releases](https://github.com/sy0u1ti/cortico-world-sound-del/releases) 下载可直接安装的 ZIP，解压后选择**包含 `package.json` 的整个文件夹**，不是 `dist` 或 `bin`。GitHub 自动生成的 Source code 压缩包只有源码，需要自行构建。

更新前先停用旧 World，确认声音已恢复，再替换或切换安装目录。不要同时启用两份；只重启旧目录的进程不会自动切换到另一个下载目录。

npm 包和发布 ZIP 已包含 Windows 辅助程序，普通用户不需要安装 Rust，也没有安装时下载、编译原生程序的脚本。

## 触发方式

插件读取主程序当前设置的快捷键，不需要再填一次。对于默认的 `LeftAlt*2` 按住模式：

- 第一次轻点不静音。
- 第二次按下并按住，直接触发静音，不等“已经开始收音”的通知。
- 松开后收到结束状态即恢复；未收到时，最多保留 350 ms 的尾部保护。

键盘监听运行在独立消息线程，回调只处理相关组合键的变化，始终放行按键，不在回调里操作音频或磁盘。快速双击的按下、松开事件不会合并丢失。

音频接口会在空闲时预热。开始静音前仍读取当前状态，先持久保存恢复清单，再设置静音并回读确认。音频工作进程由另一个进程监督；卡住时先确认旧进程结束，再最多尝试一次恢复。

支持已识别键名的一至三次按键、按住和切换模式。钩子不可用或组合含鼠标按钮时使用轮询回退。主程序通知仍用于设置同步、取消、恢复和遗漏触发兜底。

## 作用范围与隐私

- 静音所有活动播放设备，**包括 Coopanion 自己的声音**，不是只静音其他应用。
- 不修改音量数值，不修改麦克风静音。
- 原本静音的设备保持静音；检测到手动取消静音后，该轮不再争抢控制。
- 只保留本机语音面板的开关、就绪状态和快捷键信息，不记录录音、识别文本或普通键盘输入。
- 不向外部服务发送遥测，不提供模型工具，不主动调用模型。
- 恢复和诊断数据位于 `%LOCALAPPDATA%\CoopanionSoundDel`，不写到扩展安装目录。
- 恢复未确认时明确报错并保留记录，不把辅助进程退出误报成已经恢复。

普通设置页只显示单次静音上限，默认 300 秒，可设为 10–900 秒；World 列表控制启用开关。

内部配置 `worlds.sound-del.webUiUrl` 默认 `http://127.0.0.1:17788`，仅用于本机状态同步，不在普通设置里展示。非默认端口需对应修改。若本机 WebUI 需要认证，使用主程序现有的 `CORTICO_SOUND_DEL_WEB_PASSWORD` 或 `CORTICO_WEB_PASSWORD` 密钥机制。修改后重启 World，不要把凭证放进公开仓库。

## 故障恢复

辅助程序终止后需要重启这个 World；开关仍开启不等于辅助程序还在运行。

如果恢复未完成，先停用 World、接回相关设备，然后在已安装的插件目录执行：

```powershell
npm run probe
npm run recover
```

`probe` 只读检查；`recover` 会尝试恢复清单内的设备。不要与正在运行的 World 同时执行恢复，也不要删除恢复记录来消除报错。整个进程树异常退出期间的手动改动可能无法被观察到。

诊断目录中的文件：

| 文件 | 内容 |
| --- | --- |
| `last-failure.json` | 终止故障及最近的有限条操作耗时 |
| `last-slow-operation.json` | 超过 100 ms 但仍成功的操作 |
| `last-latency.json` | 检测到触发至收件、首次静音回读和首轮全部通过的耗时 |

`dispatchMs` 只代表收到请求；`firstMuteMs` 必须设置和回读成功；`allMutedMs` 必须首轮全部检查通过。没有完成的项记为 `null`，不冒充零延迟。数据区分真实与模拟后端，以及钩子、轮询、主程序通知和模拟事件。

## 使用边界

**不能保证物理上的零毫秒静音或绝对不收进开头的背景音。** 原版主程序和插件独立响应同一按键；Windows 调度、恢复落盘、声卡缓冲、蓝牙延迟和房间残响都可能影响结果。软件回读成功不等于音箱已经完全停止发声。

首次公开版本保留 `experimental` 标签：作者已经在自己的 Windows 环境试用并反馈没有明显问题，但这不等于所有驱动、蓝牙设备、独占播放和高负载组合都验证通过。[验证记录](VALIDATION.md) 同时列出通过的测试与保留的磁盘压力模拟失败。请先在自己的音乐播放与收音环境中试用。

## 开发

在独立源码目录构建，不要覆盖启用中的安装：

```powershell
npm ci
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component llvm-tools-preview,rustfmt,clippy
npm run build
npm run typecheck
npm test
npm run check:extension
npm run verify:package
```

源码位于 `src/`、`native/src/`。音频自动化测试只使用隔离模拟文件，不产生系统按键，也不操作真实播放设备。监听生命周期测试只启动未配置键的被动钩子。

发布流程见 [PUBLISHING.md](PUBLISHING.md)。许可为 [MIT](LICENSE)，第三方组件声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
