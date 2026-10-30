# 收音静音 / Sound Del

**让 Coopanion 在语音收音时自动静音后台播放，收音结束后自动恢复。**

给 Coopanion 加上类似微信输入法、网易兔八哥这类语音输入工具的「收音时后台静音」功能。听歌、看视频时，不用再手动暂停声音才能开口说话。

[English](README.md) · [下载](https://github.com/sy0u1ti/cortico-world-sound-del/releases) · [反馈问题](https://github.com/sy0u1ti/cortico-world-sound-del/issues)

## 功能

- **沿用原快捷键**：按下收音快捷键直接触发静音，不修改 Coopanion 主程序。
- **结束自动恢复**：结束或取消收音后恢复播放，原本静音的设备保持静音。
- **保留你的控制**：不改变音量大小，检测到手动取消静音后不会反复抢回控制。

## 安装

需要 **Windows x64**，以及已启用按键收音的 Coopanion 内置 `desktop-pet` 语音模块。

1. 在 **World 扩展 → 扩展市场** 搜索 `cortico-world-sound-del`。
2. 安装后按提示重启，启用「收音静音」。
3. 照常使用收音快捷键。默认 `LeftAlt*2` 是先轻点一次，再按住；第二次按下时触发静音。

市场暂时搜不到时，可从 [Releases](https://github.com/sy0u1ti/cortico-world-sound-del/releases) 下载 Windows ZIP。解压后选择**包含 `package.json` 的整个文件夹**，不是 `dist` 或 `bin`。更新前先停用旧 World。

静音覆盖所有活动播放设备，包括 Coopanion 自己的声音；不是麦克风降噪。实际响应速度受系统和声卡影响。

第三方扩展，与文中提及的输入法无关联。[设置与恢复说明](USAGE.md) · [MIT 许可](LICENSE)
