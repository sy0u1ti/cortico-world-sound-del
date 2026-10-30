# Setup And Recovery / 设置与恢复

## 设置

World 列表控制启用；单次静音上限默认 300 秒，可设为 10–900 秒。

插件通过本机语音状态接口同步快捷键、收音结束和取消状态。内部配置
`worlds.sound-del.webUiUrl` 默认 `http://127.0.0.1:17788`。只有主程序使用非默认端口时才需调整。
若 WebUI 开启认证，使用主程序的 `CORTICO_SOUND_DEL_WEB_PASSWORD`
或 `CORTICO_WEB_PASSWORD` 密钥机制；修改配置后重启 World。

## 恢复

辅助程序终止后需要重启 World。恢复未完成时，先停用 World、接回相关设备，
然后在已安装的插件目录执行：

```powershell
npm run probe
npm run recover
```

`probe` 只读检查；`recover` 尝试恢复清单中的设备，不要与启用中的 World 同时运行。
恢复数据位于 `%LOCALAPPDATA%\CoopanionSoundDel`，不要删除恢复记录来清除报错。
整个进程树异常退出期间的手动音量改动可能无法被观察到。

诊断文件为 `last-failure.json`、`last-slow-operation.json` 和 `last-latency.json`，
分别记录终止故障、慢操作和静音回读耗时。回读耗时不等于音箱停止发声的时间。

## Configuration

The World list controls enabled state. The maximum mute duration defaults to
300 seconds, configurable from 10 to 900. The internal
`worlds.sound-del.webUiUrl` follows the host's loopback port, defaulting to
`http://127.0.0.1:17788`. Authenticated hosts use `CORTICO_SOUND_DEL_WEB_PASSWORD`
or `CORTICO_WEB_PASSWORD` through their existing secret mechanism. Restart the
World after changing configuration.

## Recovery

Restart the World after a terminal helper fault. If restoration remains pending,
disable the World, reconnect the device, then run `npm run probe` and
`npm run recover` in the installed package directory. Probe is read-only;
recovery may unmute recorded devices. Never run it alongside an enabled World
or delete the recovery journal to clear an error.

Recovery and bounded diagnostics live in `%LOCALAPPDATA%\CoopanionSoundDel`.
Software mute readback is not an acoustic-silence measurement. Manual changes
made during a hard failure may be unobservable.

## More

[Validation and hardware limits](VALIDATION.md) · [Security and privacy](SECURITY.md) · [Build and publishing](PUBLISHING.md)
