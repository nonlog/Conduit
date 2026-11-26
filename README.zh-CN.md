# Conduit

[English](README.md) | 简体中文

**一款轻量、端到端加密的 Android ↔ Windows 互联工具**，专注于剪贴板同步、通知镜像、文件传输与截图/照片推送——在后台保持极低开销，长时间运行也不拖慢手机。

---

## 为什么做 Conduit？

Windows 自带的「手机连接（Phone Link）」功能齐全，但底层传输存在严重的线程与内存泄漏问题，用久了会让手机明显发热、卡顿、耗电。

Conduit 选择了一条克制的路线：不做大而全的套件，只把每天真正高频使用的几项互联能力做到稳定、可靠、无感，并把**后台轻量**作为第一优先级：

- **Android 端近乎零待机开销** — 不轮询、不定时扫描网络、不持有长时唤醒锁，网络切换时不反复重建会话。
- **Windows 端原生轻量常驻** — 常驻的 Rust 后台守护进程 + 按需启动的原生桌面界面（`conduit-rune`），内存、句柄与线程数长期受控。
- **简洁可控的连接生命周期** — 优先局域网直连，离网自动切换加密中继；不引入 ICE / STUN / TURN 等复杂且易泄漏的组件。

> **刻意不包含的功能：** 通话与短信、投屏镜像、远程键鼠控制、媒体播放控制、远程文件系统挂载。

---

## 核心功能

- **双向剪贴板同步** — 手机与电脑间自动同步文字与图片。已 Root 设备支持通过 LSPosed 无感后台读取；未 Root 设备提供基于无障碍服务的事件驱动方案（无轮询）。
- **手机通知 → Windows 原生通知** — 将 Android 通知镜像为 Windows 原生 Toast，支持通知更新、消除同步、快捷操作按钮以及直接回复。
- **双向文件传输** — 手机端通过系统分享菜单发送（支持 Direct Share 直接显示已配对电脑名），电脑端通过资源管理器或 Windows 系统「共享」目标发送。
- **照片与截图即时推送** — 手机新拍的照片和截屏会自动推送到 Windows 通知，可选一键调用系统「截图工具」继续标注或复制。
- **网页链接互传** — 在手机与电脑之间一键推送并打开网页。
- **多台电脑管理** — 一部手机可同时配对并连接多台 Windows 电脑，在 **设置 → 设备** 中独立管理。
- **局域网直连 + 端到端加密中继** — 同局域网优先通过 mDNS 直连（`TCP 41112`），外出使用流量或跨网络时自动走盲转中继（`TCP 41113`）。所有通信均采用 Noise 协议（`Noise_XX_25519_ChaChaPoly_BLAKE2s`）端到端加密，中继服务器只能看到密文字节，无法读取任何内容。

---

## 快速开始

### 系统要求

- **Android：** Android 10 及以上（API 29+）
- **Windows：** Windows 10 / 11（64 位）

### 1. 安装 Windows 端

前往 [GitHub Releases](https://github.com/nonlog/Conduit/releases/latest) 下载最新的 Windows 压缩包解压运行，或通过 [Scoop](https://scoop.sh) 从 [`nonlog/scoop-www`](https://github.com/nonlog/scoop-www) 仓库安装：

```powershell
scoop bucket add www https://github.com/nonlog/scoop-www
scoop install www/conduit
```

### 2. 安装 Android 端

前往 [GitHub Releases](https://github.com/nonlog/Conduit/releases/latest) 下载并安装签名版 APK。

### 3. 配对设备

1. 在 Windows 和 Android 上分别打开 **Conduit**。
2. 在 Windows 端点击配对，界面会显示 6 位配对码和二维码。
3. 在 Android 端扫码或输入 6 位配对码，核对双端显示的安全指纹一致后即可完成配对。

### 4. 开启 Android 后台剪贴板同步

Android 10+ 限制后台应用读取剪贴板，Conduit 提供两种零轮询方案：

- **已 Root 设备（推荐，LSPosed）：** 在 LSPosed 中启用 Conduit 模块，作用域勾选 **系统框架（System Framework）** 与 **Conduit** 后重启，Conduit 会自动启用低开销的 Hook 通道。
- **未 Root 设备（原厂系统）：** 按应用内引导开启「Conduit 剪贴板同步」无障碍服务。该服务仅在已连接电脑时响应系统复制事件，断连后自动休眠，不进行任何后台轮询。

---

## 项目结构

| 目录 | 技术栈 | 说明 |
|---|---|---|
| `android/` | Kotlin、Jetpack Compose | Android 客户端、通知监听与可选 LSPosed 剪贴板模块 |
| `windows/conduit-daemon/` | Rust | 轻量级 Windows 常驻后台守护进程 |
| `windows/conduit-rune/` | Rust | 按需启动的原生 Windows 桌面端界面 |
| `windows/conduit-share-target/` | Rust | 原生 Windows 系统「共享」目标宿主 |
| `relay/` | Rust | 无状态盲转 TCP 中继服务，用于非局域网环境连接 |
| `proto/` | Protocol Buffers | 跨端共享的通信协议定义（唯一事实来源） |

---

## 开发与文档

如需深入了解架构细节或参与开发，请参阅以下文档（英文）：

- [架构说明](docs/architecture.md) — 传输层、Noise XX 握手、帧格式与安全模型
- [开发指南](docs/development.md) — 构建流程、GitHub Actions CI 与开发版安装
- [设计决策记录](docs/decisions.md) — 协议与各平台关键取舍的背景说明
- [待办与验证清单](docs/TODO.md) — 当前进展与长稳测试清单

Android、Windows 与 Relay 的正式发布包均通过 GitHub Actions 在干净环境中可复现构建，并附带 `SHA256SUMS.txt` 发布于 [GitHub Releases](https://github.com/nonlog/Conduit/releases)。

---

## 说明与致谢

Conduit 为完全独立的净室实现。在协议与架构设计阶段参考了 [KDE Connect](https://kdeconnect.kde.org/) 与 [Sefirah](https://github.com/shrimqy/Sefirah) 的思路，未复制任何代码。
