# Satelite

A lightweight and modern sing-box GUI client for macOS, built with Tauri, React and Rust. Supports Clash subscriptions, TUN, DNS, routing rules and system proxy.


<p align="center">
  <img src="assets/index.png" alt="Satelite" width="720" />
</p>

轻量级 **sing-box** 桌面代理客户端，基于 **Tauri 2 + React + Rust**。

面向 macOS（优先 arm64）日常使用：订阅导入、节点切换、规则分流、DNS、系统代理 / TUN，以及托盘常驻。


## 低价直连  〢  [良心云](https://xn--9kqz23b19z.com/#/register?code=SUpAMRXL)  〢  [一分机场](https://xn--4gqx1hgtfdmt.com/#/register?code=qiRqViZ8)  〢  [赔钱机场](https://xn--cp3a08l.com/register?code=lzWSCUqr&cover=sfw)

**简评** : 量大管饱，1000G流量不限时，最具性价比

---

## 功能

| 模块 | 说明 |
|------|------|
| **概览** | 启停代理、当前节点、出站模式（规则 / 全局 / 直连）、系统代理与 TUN、流量与连接摘要 |
| **配置** | Clash 订阅：URL 或本地文件导入、更新、删除；解析后持久化到 app data |
| **节点** | 节点列表、延迟测试、切换当前节点（运行中热切换） |
| **规则** | 分流规则管理；内置规则集 |
| **DNS** | System / Smart / Custom 等模式，DoH / DoT、FakeIP、白名单 |
| **连接 / 请求 / 日志** | 运行时连接与日志查看 |
| **设置** | 端口、开机启动、关窗到托盘、语言、外观（Day / Mission）等 |
| **托盘** | 打开界面 / 启停 / 退出清理 |

**协议支持：** Shadowsocks · VMess · VLESS · Trojan · Hysteria2 · TUIC · SOCKS5

**内核：** 可内置 macOS arm64 的 sing-box；也可从 GitHub Release 下载到 app data（下载版优先于内置）。

---

## 技术栈

- **桌面壳：** Tauri 2
- **前端：** React 19 + TypeScript + Vite
- **后端：** Rust（订阅解析、配置生成、CoreManager、系统代理等）
- **代理内核：** [sing-box](https://github.com/SagerNet/sing-box)
- **存储：** JSON（`store.json`），后续可考虑 SQLite

---

## 环境要求

- macOS（当前重点支持 **darwin-arm64**）
- [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/) / Cargo
- Xcode Command Line Tools（Tauri 构建需要）

---

## 开发

```bash
# 安装前端依赖
pnpm install

# 如缺少内置内核（可选）
./scripts/fetch-bundled-core-darwin-arm64.sh

# 后端测试
cd src-tauri && cargo test && cd ..

# 启动开发模式（前端 + Tauri）
pnpm tauri dev
```

开发时默认 Vite 地址：`http://localhost:1420`。

**建议上手路径：** 配置页 → 添加配置（URL 或本地文件）→ 解析成功后启动代理 → 在节点页或概览切换节点。

---

## 打包

生成 macOS `.dmg`：

```bash
./scripts/build-dmg.sh
```

脚本会检查内置 sing-box、安装依赖，并执行 `pnpm tauri build --bundles dmg`。产物一般在：

```text
src-tauri/target/release/bundle/dmg/
```

---

## 数据流

```text
URL 下载 / 本地文件
        │
        ▼
 parse_subscription
        │
        ▼
   ProxyNode[]
        │
        ▼
  store.json（订阅、节点、设置等）
        │
        ▼
  sing-box 配置生成 → config/active.json
        │
        ▼
  CoreManager 启动内核 + clash_api
        │
        ├── 系统代理（macOS networksetup）
        └── TUN（可选）
```

---

## 目录结构（简要）

```text
satelite/
├── src/                 # React 前端
│   ├── pages/           # 各功能页
│   ├── components/      # 侧栏、导航等
│   ├── theme/           # Day / Aerospace 主题
│   └── i18n/            # 中英文
├── src-tauri/           # Rust + Tauri
│   ├── src/             # 命令、领域模型、内核、配置构建
│   └── resources/       # 内置 sing-box、规则、DNS 资源
├── docs/                # 设计与 PRD
└── scripts/             # 内核拉取、DMG 打包等
```

---

## 状态说明

当前版本功能已覆盖订阅、内核启停、系统代理 / TUN、规则、DNS、托盘与多页面 UI。存储仍为 JSON；流量页等能力会随迭代继续完善。

---

## 许可

见仓库中的 `LICENSE`（若已添加）。
