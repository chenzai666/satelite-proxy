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
| **体积与内存** | Tauri 2 + Rust，不是 Chromium 全家桶。开着托盘就该被忘掉，而不是占掉半条内存。 |
| **节点会挂** | 三种选路：手动、内核 urltest、应用侧智能切换。智能模式靠连接日志被动感知 + 按需探测，自动避障，而不是一直狂扫全表。 |
| **不想被配置淹没** | 「简洁模式」只留连接 / 节点 / 流量；「专业模式」打开规则、DNS、Hosts、日志。同一套内核，两套节奏。 |
| **界面也是功能** | 玻璃拟态、浅色 / 深色、多种主题色。打开窗口的那一秒，就该知道这不是 2018 年的后台面板。 |
| **开箱即用** | 内核自动下载更新；Clash 订阅、文件导入、`clash://` / `sing-box://` 浏览器一键导入。 |

> 卫星绕着你转，而不是你围着 YAML 转。

---

## 它能做什么

- **订阅导入**：Clash 兼容订阅，链接 / 文件 / 浏览器深链一键导入
- **协议齐全**：SS、VMess、VLESS、Trojan、Hysteria2、TUIC、SOCKS5、AnyTLS、Snell，一键测速，秒切节点
- **智能选路**：手动 · 应用智能避障 · 内核 urltest，按场景选，不绑死一种策略
- **规则分流**：Rule Set 分流，远程规则集自动缓存；规则 / 全局 / 直连一键切换
- **智能 DNS**：DoH / DoT / FakeIP，自定义 DNS 规则与 Hosts，还能测解析
- **系统代理 / TUN**：系统代理一键接管；TUN 全局透明代理，支持 system / gvisor / mixed
- **连接与流量**：活跃连接、已关闭、失败请求、流量走向，自动解析进程名
- **托盘常驻**：关窗即托盘，开机启动、静默启动；内核在后台，窗口可消失
- **内核自管**：自动拉取并更新 sing-box，不用自己找二进制、对版本
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

- **内核**：[sing-box](https://github.com/SagerNet/sing-box)
- **桌面框架**：[Tauri 2](https://tauri.app/)
- **前端**：React + TypeScript + Vite
- **后端**：Rust

## 📦 开发

```bash
# 安装依赖
pnpm install

# 下载打包资源（sing-box 内核 + 三条内置远程规则集 .srs，缺省时应用会回退到在线下载）
./scripts/fetch-bundled-rule-sets.sh

# 启动开发模式
pnpm tauri dev

# 打包构建
pnpm tauri build
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
pwsh scripts/build-windows.ps1
```

也可以直接推送到 `main` 分支，GitHub Actions 会自动构建 Windows NSIS 安装包。构建完成后进入仓库的 **Actions → Windows 构建**，在任务页面底部下载 `Satelite-Windows-*` 构建产物；云端产物保留 14 天，也支持在 Actions 页面手动触发构建。

正式发布时推送以 `v` 开头的版本标签（例如 `v1.0.8-fork.1`），工作流会在构建成功后自动创建 GitHub Release，并把 Windows 安装包上传到 Release 资产。

### 局域网代理

开启“允许局域网连接”后，主 mixed 入站会监听 `0.0.0.0:2080`（端口以应用设置为准）。局域网设备应把 HTTP/SOCKS 代理地址设置为 Satelite 所在电脑的局域网 IPv4 和该端口。Windows 用户还需要在防火墙提示中仅允许 `sing-box.exe` 访问专用网络；不要向公用网络或互联网开放该端口。

---

用着顺手的话，点一颗 [Star](https://github.com/zn0wii/satelite-proxy)，卫星会飞得更稳一点。

## 友情链接

- **佬友聚集地** [linux.do](https://linux.do/)
