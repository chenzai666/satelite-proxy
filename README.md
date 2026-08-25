# Satelite

<p align="center">
  <strong>卫星飞天，连接无限。</strong><br/>
  轻量、好看的 <a href="https://github.com/SagerNet/sing-box">sing-box</a> 桌面客户端<br/>
</p>

<p align="center">
  <a href="https://github.com/zn0wii/satelite-proxy/stargazers"><img src="https://img.shields.io/github/stars/zn0wii/satelite-proxy?style=social" alt="Stars" /></a>
  &nbsp;
  <img src="https://img.shields.io/badge/macOS-Apple%20Silicon%20%7C%20Intel-111111?logo=apple&logoColor=white" alt="macOS" />
  <img src="https://img.shields.io/badge/Windows-x64-0078D4?logo=windows&logoColor=white" alt="Windows" />
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri" />
  <img src="https://img.shields.io/badge/Rust-%23000000?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/License-Apache%202.0-green.svg" alt="License" />
</p>

导入订阅、切节点、规则分流、智能 DNS、系统代理 / TUN、托盘常驻——日常该有的都有。  
它**足够轻、足够稳、也足够好看**。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/banner-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="./assets/banner-light.png">
    <img src="./assets/banner-light.png" alt="Banner">
  </picture>
</p>

## 为什么是 Satelite

代理客户端已经够多了。Satelite 不想再做一个「功能清单更长」的壳，而是把 **sing-box** 收成一颗真正能放在桌面上的卫星：

| 你真正在意的 | Satelite 怎么做 |
| --- | --- |
| **体积与内存** | Tauri 2 + Rust，不是 Chromium 全家桶。开着托盘就该被忘掉，而不是占掉半条内存。关到托盘还可选「低内存模式」，把界面卸掉。 |
| **节点会挂** | 三种选路：手动、内核 urltest、应用侧智能切换。智能模式靠连接日志被动感知 + 按需探测，自动避障，而不是一直狂扫全表。 |
| **不想被配置淹没** | 「简洁模式」只留连接 / 节点 / 流量；「专业模式」打开规则、DNS、Hosts、日志。同一套内核，两套节奏。 |
| **界面也是功能** | 玻璃拟态、浅色 / 深色、多种主题色、首页动效三选一（粒子 / 经典 / 笑脸）。打开窗口的那一秒，就该知道这不是 2018 年的后台面板。 |
| **开箱即用** | 内核自动下载更新；Clash 订阅、sing-box JSON、分享链接、`clash://` / `sing-box://` / `singbox://` 浏览器一键导入。 |

> 卫星绕着你转，而不是你围着 YAML 转。

---

## 它能做什么

- **订阅与配置**：Clash 订阅、V2rayN Base64、sing-box JSON、节点分享链接；链接 / 文件 / 浏览器深链导入；订阅可定时更新。解析后的订阅节点可逐项编辑，刷新时会继续套用本地修改。也可以把一份完整 sing-box 配置直接当运行时。
- **协议**：SS、VMess、VLESS、Trojan、Hysteria2、TUIC、AnyTLS、SOCKS5 等，一键测速，秒切节点
- **智能选路**：手动 · 应用智能避障 · 内核 urltest，按场景选，不绑死一种策略
- **规则分流**：多规则集（本地 / 远程 sing-box JSON、`.srs`、Clash `.list`），拖拽优先级；内置国内站点 / 国内 IP / 海外规则；兜底出口代理 / 直连 / 屏蔽。规则 / 全局 / 直连一键切换
- **DNS 与 Hosts**：DoH / DoT / FakeIP，DNS 规则集、系统 Hosts、默认解析器，还能测解析
- **系统代理 / TUN**：系统代理一键接管；TUN（system / gvisor / mixed）；绕过局域网、可选 TUN IPv6、可拦 QUIC
- **端口**：mixed / Clash API、多监听、允许局域网
- **连接与流量**：活跃连接、已关闭、失败请求、流量走向，自动解析进程名；切换节点可自动中断旧连接，流量页也可手动关闭全部连接
- **托盘常驻**：关窗进托盘，开机启动、静默启动、可选托盘图标；内核在后台，窗口可消失
- **多内核**：支持 sing-box、Mihomo 与 Xray；可在设置中切换，并按所选内核过滤不兼容节点
- **内核自管**：自动拉取并更新内核，不用自己找二进制、对版本
- **中英双语文案**，浅色 / 深色，多种主题色

<p align="center">
  <img src="assets/1.png" alt="Windows 概览" width="760" />
  &nbsp;
  <img src="assets/3.png" alt="规则分流" width="760" />
</p>
<p align="center">
  <img src="assets/2.png" alt="应用设置" width="760" />
</p>

## 🖥 平台支持

| 平台            | 状态   |
| --------------- | ------ |
| macOS Apple 芯片 | ✅ 支持 |
| macOS Intel     | ✅ 支持 |
| Windows         | ✅ 支持 |
| Linux           | 🚧 计划中 |

> Satelite Proxy 仍在持续开发中，升级前请备份重要的配置文件。

## 🛠 技术栈

- **代理内核**：[sing-box](https://github.com/SagerNet/sing-box)、[Mihomo](https://github.com/MetaCubeX/mihomo)、[Xray](https://github.com/XTLS/Xray-core)
- **桌面框架**：[Tauri 2](https://tauri.app/)
- **前端**：React + TypeScript + Vite
- **后端**：Rust

## 📦 开发

```bash
# 安装依赖
pnpm install

# 启动开发模式（缺内核或内置规则集时，应用会自行下载）
pnpm tauri dev
```

打包脚本会拉取对应平台的官方 sing-box，以及三条内置远程规则集（`.srs`）。也可以先手动放进 `src-tauri/resources/`：

```bash
# macOS Apple Silicon / Intel
./scripts/fetch-bundled-core-darwin-arm64.sh    # 或 fetch-bundled-core-darwin-amd64.sh
./scripts/fetch-bundled-rule-sets.sh
```

```powershell
# Windows x64
pwsh scripts/fetch-bundled-core-windows-amd64.ps1
# 规则集由 build-windows.ps1 一并拉取
```

### macOS DMG

在 **macOS** 上执行（不必在本机对应架构上编：Apple 芯片可交叉编 Intel）：

```bash
# 按本机架构打包
./scripts/build-dmg.sh

# Apple Silicon
./scripts/build-dmg.sh --arch arm64

# Intel（x86_64）
./scripts/build-dmg.sh --arch intel
# 等价：
./scripts/build-dmg-intel.sh
```

脚本会拉取对应架构的官方 sing-box 内核并打进安装包。产物在：

`src-tauri/target/<aarch64|x86_64>-apple-darwin/release/bundle/dmg/`

### Windows 安装包

```powershell
pwsh scripts/build-windows.ps1              # NSIS 安装包（默认）
pwsh scripts/build-windows.ps1 -Bundle msi  # MSI
```

产物在 `src-tauri/target/release/bundle/nsis/` 或 `.../msi/`。

也可以直接推送到 `main` 分支，GitHub Actions 会自动构建 Windows NSIS 安装包。构建完成后进入仓库的 **Actions → Windows 构建**，在任务页面底部下载 `Satelite-Windows-*` 构建产物；云端产物保留 14 天，也支持在 Actions 页面手动触发构建。

正式发布时推送与应用版本一致、以 `v` 开头的标签（例如 `v1.0.14`），工作流会在构建和测试成功后自动创建 GitHub Release，并把 Windows 安装包上传到 Release 资产。

### 局域网代理

开启“允许局域网连接”后，主 mixed 入站会监听 `0.0.0.0:2080`（端口以应用设置为准）。局域网设备应把 HTTP/SOCKS 代理地址设置为 Satelite 所在电脑的局域网 IPv4 和该端口。Windows 用户还需要在防火墙提示中仅允许 `sing-box.exe` 访问专用网络；不要向公用网络或互联网开放该端口。

### Windows 系统代理与 Clash 规则

Windows 系统代理会同时更新注册表、当前 LAN 连接以及已有拨号/VPN 连接，并在规则模式重启内核后重新通知系统应用，减少浏览器、ChatGPT 和软件下载器继续使用旧代理的情况。

规则模式下，`cliproxy.yu8.lat`（兼容旧地址 `cpa.yu8.lat`）使用“直连优先、代理回退”：内核每分钟通过直连和当前代理组探测反代首页，直连可用时保持直连，直连失败时自动切换到当前手动节点。OpenAI 与 ChatGPT 官方域名仍按 AI 规则走代理。该机制同时适用于系统代理和 TUN，不需要把整个应用切到全局模式。

远程规则集可直接填写 sing-box source JSON、二进制 `.srs` 或 Clash classical `.list` / `payload:` YAML；Clash 列表会在本地转换为 sing-box source JSON。列表里的第三列策略不会被信任，实际出口仍以 Satelite 中该规则集选择的“代理 / 直连 / 屏蔽”为准。

### 编辑订阅节点

在“配置”页面打开订阅卡片右上角菜单，选择“编辑节点”，即可修改解析后的节点名称、服务器、端口、协议、认证、TLS / REALITY 与传输参数。修改只保存在本机，不会写回远程订阅；普通刷新时，只要远程订阅中仍存在同一源节点，本地修改就会自动重新套用。若服务商删除或替换了该节点，对应本地修改也会自然失效，避免错误覆盖新节点。

### UDP 节点兼容模式

TUIC 节点在订阅未提供 ALPN 时会自动补充标准的 `h3`。如果可信订阅中的 Hysteria2/TUIC 节点仍提示“证书不受信任”，可在“设置 → 端口与网络”开启“UDP 节点兼容模式”。该模式仅对这两类协议设置 `insecure=true`，不会改动 VLESS、Shadowsocks 等其他节点。

> 此模式会跳过服务器证书验证，存在中间人攻击风险。应优先要求节点服务商修复证书链，并且不要对来源不明的订阅启用。

---

用着顺手的话，点一颗 [Star](https://github.com/zn0wii/satelite-proxy)，卫星会飞得更稳一点。

## 友情链接

- **佬友聚集地** [linux.do](https://linux.do/)
