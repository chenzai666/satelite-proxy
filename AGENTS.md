# AGENTS.md — Satelite Proxy 项目地图

面向 AI agent 的项目速查文档。读完本文即可定位绝大多数代码，无需重复探索。
最后核对：2026-08-23（v1.0.9）。

## 0. 阅读与维护规则（必读）

**对 agent 的要求**：在本仓库做任何改动前，先通读本文档——尤其是「§1 快速上手」、§7 修改场景速查表、§9 约定与坑。不要凭猜测探索全库。

**文档同步规则**：项目发生重大变动时，**必须与代码同一次提交同步更新本文档**，包括但不限于：

| 变动类型 | 需更新的章节 |
|---|---|
| 新增 / 删除 / 移动模块或源文件 | §3 目录速览、§5/§6 对应模块详解 |
| 新增 / 改名 command 或 Tauri 事件 | §5.1（注册表）、§5.8、§6.2 |
| 数据模型 / 存储结构 / 磁盘布局变化 | §5.2、§5.3 |
| 配置生成或内核管理方式变化 | §5.4、§5.5 |
| 构建 / 打包 / 测试流程变化 | §1 快速上手、§8 |
| 新的平台分支、新的坑 | §9 |

小改动（文案、bugfix、样式微调）不强制更新；文中行数标注允许过时，以「文件存在性与职责描述」为准，发现明显过时顺手修正并更新文首「最后核对」日期。

## 1. 快速上手：环境 · 编译 · 测试 · 打包

### 环境要求

- **Node + pnpm**（registry 已锁定 npmmirror，见 `.npmrc`；依赖只能用 pnpm 装）
- **Rust stable**：Windows 需 MSVC 工具链（build 脚本用 vswhere 检测）；macOS 需 Xcode CLT
- 平台限制：DMG 只能在 macOS 打，Windows 安装包只能在 Windows 打；Apple Silicon 可交叉编 Intel（脚本自动 `rustup target add x86_64-apple-darwin`）

### 开发调试

```bash
pnpm install        # 安装前端依赖
pnpm tauri dev      # 一键启动 Rust 后端 + WebView（Vite 端口 1420 strictPort）
```

- 前端改动走 HMR；Rust 改动自动重编并重启应用
- **不要用 `pnpm dev` 调 UI**——只起 Vite 没有后端，所有 `invoke` 会失败；UI 调试也用 `pnpm tauri dev`
- 首次 dev 缺 sing-box 内核 / 内置规则集会**自动联网下载**；离线环境先跑下面的「资源预取」
- 深链调试： schemes 为 `clash://` `sing-box://` `singbox://`（Windows/Linux dev 下启动时自动注册）

### 检查与测试

```bash
pnpm build                                        # 前端：tsc 严格类型检查 + vite 产物（提交前必过）
cd src-tauri && cargo check                       # Rust 快速检查
cd src-tauri && cargo test                        # Rust 全部测试（含散落 #[cfg(test)] 单测）
cd src-tauri && cargo test --test parse_subscription          # 只跑订阅解析集成测试
cd src-tauri && cargo test --test download_core_live -- --ignored  # 真网下载 live 测试（慢，慎跑）
cd src-tauri && cargo fmt / cargo clippy          # 标准 Rust 工具链
```

- 前端**没有** ESLint/Prettier 配置，质量门槛 = `tsc` strict + `pnpm build`
- 测试 fixtures 在 `src-tauri/tests/fixtures/`（clash yaml ×2、singbox json ×1）

### 打包发布

```bash
# macOS DMG（产物: src-tauri/target/<aarch64|x86_64>-apple-darwin/release/bundle/dmg/）
./scripts/build-dmg.sh              # 按本机架构
./scripts/build-dmg.sh --arch arm64
./scripts/build-dmg.sh --arch intel # 交叉编译；等价 build-dmg-intel.sh

# Windows（产物: src-tauri/target/release/bundle/nsis/ 或 .../msi/）
pwsh scripts/build-windows.ps1              # NSIS 安装包（默认）
pwsh scripts/build-windows.ps1 -Bundle msi  # MSI
```

打包脚本会自动拉取对应平台的官方 sing-box 并打进安装包，无需手动准备。

### 资源预取（可选，离线/加速用）

```bash
scripts/fetch-bundled-core-darwin-arm64.sh        # macOS arm64 sing-box（默认 v1.13.18）
scripts/fetch-bundled-core-darwin-amd64.sh        # macOS Intel
pwsh scripts/fetch-bundled-core-windows-amd64.ps1 # Windows sing-box v1.13.15 + libcronet.dll，支持 -Proxy
scripts/fetch-bundled-xray-darwin-arm64.sh        # macOS arm64 Xray（默认 v26.3.27）+ geosite/geoip.dat
scripts/fetch-bundled-xray-darwin-amd64.sh        # macOS Intel Xray
pwsh scripts/fetch-bundled-xray-windows-amd64.ps1 # Windows Xray + geodata + wintun.dll（TUN 用），支持 -Proxy
scripts/fetch-bundled-rule-sets.sh                # 3 条内置 .srs 规则集（校验 SRS 魔数，--force 重下）
scripts/memory-profile/                           # WebView2 内存剖析（CDP 堆采样 + 进程树 RSS，见其 README 与 docs/webview2-memory-optimization-plan.md）
```

- 这些二进制**不入 git**（`.gitignore` 排除 `resources/bin/**/sing-box*`、`xray*`、`*.dat`、`wintun.dll`、`libcronet.*`、`resources/rule-sets/*.srs`），本地缺失属正常
- 图标再生成：`python scripts/generate-icons.py`（依赖 Pillow，产出应用图标 + 8 种托盘图标）

## 2. 项目是什么

**Satelite**（`com.satelite.proxy`）— 轻量级桌面代理客户端，Tauri 2 桌面应用，支持**双内核**。

- **内核**：sing-box（默认）与 Xray（`settings.core_type` 全局切换），均作为 bundled resource 随应用分发（**不是** Tauri sidecar；由应用代码解压/下载/拉起；Xray 另需 geosite.dat/geoip.dat，Windows TUN 需 wintun.dll）
- **后端**：Rust（`src-tauri/`），负责订阅解析、双内核配置生成、内核生命周期、系统代理、托盘、规则/DNS/连接数据
- **前端**：React 19 + TS + Vite（`src/`），玻璃拟态 UI，无路由库、无状态管理库、无 CSS 框架
- **平台**：macOS (arm64/amd64) + Windows x64；Linux 计划中
- **包管理**：pnpm；前端端口 1420（strictPort）
- 语言：UI 中英双语（zh 默认）；代码注释中英混合

## 3. 目录速览

```
satelite-proxy/
├── src/                     # React 前端（~26.5k 行）
│   ├── api.ts               # ★ 前后端唯一桥：全部 invoke 封装（726 行）
│   ├── types.ts             # ★ 前端共享类型（与 Rust domain 对应，538 行）
│   ├── App.tsx              # Provider 栈 + ProShell/SimpleShell 切换
│   ├── pages/               # 专业模式 12 个页面
│   ├── ui/simple/           # 简洁模式 UI（独立 shell + 4 页）
│   ├── components/          # 玻璃设计系统 + 3D 首页 + 弹窗表单
│   ├── hooks/               # useVirtualRange / useVisibleInterval / 拖拽排序等
│   ├── i18n/                # zh/en 扁平文案表（TS 强制双语言键一致）
│   ├── theme/               # aerospace 深色 / day 浅色 + 6 主题色
│   └── App.css              # ★ 全部样式单文件（~7.6k 行，按 /* —— 段落 —— */ 分节）
├── src-tauri/               # Rust 后端（~36k 行）
│   ├── src/lib.rs           # ★ 入口：setup 流程 + 全部 command 注册
│   ├── src/commands/        # Tauri command 分层（按域拆文件）
│   ├── src/domain/          # ★ 核心数据模型（node/rule/dns/settings/subscription）
│   ├── src/state.rs         # AppState：全局状态中枢（1321 行）
│   ├── src/storage/store.rs # AppStore 持久化（JSON，含备份/迁移，2666 行）
│   ├── src/config/          # 配置生成：builder.rs（sing-box）+ xray.rs（Xray）+ dns_build/write/…
│   ├── src/core/            # 内核进程管理：kind.rs（CoreKind 双内核描述）、manager/download/assets/paths/提权/Job Object
│   ├── src/runtime.rs       # 编排：config→core→system proxy（~1600 行，含 Xray 分支）
│   ├── src/api/clash_api.rs # Clash API 客户端（ureq + tungstenite）
│   ├── src/api/xray_metrics.rs # Xray metrics 客户端（/debug/vars 轮询）
│   ├── src/subscription/    # 订阅解析（clash/singbox/uri/manual）
│   ├── src/proxy/           # 系统代理（windows.rs / macos.rs / stub.rs）
│   ├── src/tray.rs          # 托盘
│   └── tauri.conf.json      # 主配置 + windows/macos-intel 覆盖
├── scripts/                 # 构建脚本（拉双内核/规则集、DMG、NSIS/MSI、图标生成）
└── src-tauri/tests/         # 集成测试（订阅解析 fixtures + live 下载测试）
```

## 4. 架构与数据流

```
React UI ──invoke()──▶ commands/* ──▶ AppState ──▶ storage(磁盘 JSON store)
                          │              │
                          │              ├─▶ config/builder.rs (sing-box) 或 config/xray.rs (Xray)
                          │              │      按 settings.core_type 生成 ─▶ <data>/config/active.json
                          │              ├─▶ core/* 拉起内核进程 (两内核均为 run -c active.json)
                          │              └─▶ proxy/* 设置系统代理 (Win registry / macOS networksetup)
                          ▼
       sing-box 模式: api/clash_api.rs ◀──(HTTP/WS, ureq)── clash_api（连接快照/流量/延迟/选节点）
       Xray 模式:    api/xray_metrics.rs ◀──(HTTP, ureq)── metrics /debug/vars（仅流量总量；无逐连接/选节点 API，切节点=重启）
                          │
                          ▼
              conn_journal / state 缓存 ──invoke 轮询──▶ React UI
```

关键事实：

- **双内核**：`settings.core_type`（`singbox` 默认 | `xray`）决定配置生成器与二进制；两套生成器共享 domain 模型、互不依赖（v2rayN 同款模式）。Xray 模式下切节点/切规则 = 重写配置重启进程；连接三页面无数据（前端显示占位提示）。
- **前端不直连 Clash API**。`src/` 里零 fetch/WebSocket，全部经 Rust command 中转；实时数据靠 `useVisibleInterval` 轮询 invoke + 5 个 Tauri 事件。
- **单窗口应用**。专业/简洁模式复用同一窗口，尺寸切换（960×720 ↔ 420×720）见 `src/ui/windowLayout.ts` 与 `src-tauri/src/window_ctrl.rs`。
- **无路由**。导航是 `App.tsx` 里 `useState<NavKey>` + `TopNav`；次级页面 `React.lazy`（WebView 低内存重建）。
- **关窗进托盘**：`CloseRequested` 被拦截（`lib.rs:322-347`），可销毁 WebView 省内存但保活 Rust/tray/内核；`exit_allowed` 标志控制真正退出。

## 5. 后端模块详解（src-tauri/src/）

### 5.1 入口与生命周期

- `lib.rs` — `run()`：插件注册（opener/dialog/deep-link/single-instance）→ setup（加载 store 失败则弹窗退出）→ 托盘 → 启动 5 个后台任务 → 深链处理 → 静默启动/自动代理恢复。**全部 ~80 个 command 在 `lib.rs:348-431` 注册**，实现在 `commands/*.rs`（`commands/mod.rs` re-export）。
- 后台任务（均在 setup 中 spawn）：
  - `conn_journal.rs` — 轮询/WS 订阅 Clash 连接快照（UI 可见时 100ms，托盘时降频），维护活跃+历史连接环形日志
  - `subscription_auto.rs` — 按 `auto_update` 间隔定时刷新订阅（默认 1440 分钟）
  - `remote_rule_auto.rs` — 应用侧下载远程规则集缓存到本地，sing-box 只加载本地文件
  - `smart_switch.rs` — 智能选路：被动连接日志感知劣化 → 按需探测 top-K 候选 → 评分+容差+冷却
  - `rule_apply.rs` — 规则变更的 500ms 防抖合并 + 全局串行 apply-and-restart
- `main.rs` — 仅调 `run()`。

### 5.2 状态与存储

- `state.rs` — `AppState`（managed state，`Mutex<AppStore>` + runtime 句柄 + pending 深链 URL + UI 可见标志）。几乎所有 command 走 `state.with_store(...)` / `with_store_mut(...)`。
- `storage/store.rs` — `AppStore`（serde JSON）：`subscriptions`、`nodes`（StoredNode）、`settings`、`dns`、`rule_sets`、`node_aliases` + 4 组 `retained_*`（**解析不了的新 schema 数据写回而非丢弃**）。含 `store.backup.json` 备份、损坏快照保留、schema 迁移（如 capture_mode/auto_select 迁移）。
- 磁盘布局（`app_data_dir`）：
  - `store.json` — 主存储；`store.backup.json` — 备份
  - `config/active.json` — 生成的 sing-box 运行配置（`config/write.rs`，tmp+rename 原子写，带时间戳备份）；custom 运行时另有独立文件，**绝不写 active.json**
  - `bin/sing-box(.exe)` + `version.txt` — sing-box 内核；`bin/xray(.exe)` + `xray-version.txt` — Xray 内核；`bin/geosite.dat`/`geoip.dat`/`wintun.dll` — Xray 资产（`core/paths.rs` + `core/assets.rs`）
  - `logs/` — 应用日志（`app_log.rs`，`log_retention.rs` 清理）
  - 远程规则集缓存（`.srs`）

### 5.3 数据模型（domain/）★ 改功能先看这里

| 文件 | 内容 |
|---|---|
| `domain/node.rs` | `Protocol`（SS/VMess/VLESS/Trojan/Hysteria2/TUIC/AnyTLS/SOCKS5…）、`ProtocolConfig`、`TlsConfig`、`Transport`、`ProxyNode`、`ParseResult`、`ManualNodeDraft`（表单模型） |
| `domain/settings.rs` | `AppSettings`（~40 字段：端口/TUN/capture_mode/outbound_mode/auto_select/locale/theme/accent/hero_style/tray_icon…）、`OutboundMode`、`CaptureMode`、`AutoSelectMode`、`ExtraInbound`、`RuntimeSource` |
| `domain/rule.rs` | `Rule`、`RuleSet`（本地/远程/内置，ownership/strategy/dns_strategy；strategy 6 值 proxy/direct/block/node/filter/smart，node=整组指定节点、filter=整组关键词过滤池，参数存集级 node_id/smart_include 等字段）、`RuleType`、`RuleTarget`、`BUILTIN_REMOTE_RULE_SETS`（3 条内置远程规则，需与 `scripts/fetch-bundled-rule-sets.sh` 同步） |
| `domain/dns.rs` | `DnsSettings`、`DnsRule`、`DnsAction`、FakeIP、Hosts 配置 |
| `domain/subscription.rs` | `Subscription`、`SubscriptionSource`（url/file/text/node/singbox）、`SubscriptionView` |

### 5.4 配置生成（config/）★ 两套独立生成器共享 domain 模型

- `builder.rs` — ★ sing-box 生成器：`ProxyNode[] + AppSettings + RuleSets + DnsSettings → sing-box JSON`（`BuildOptions`）。inbounds（mixed/Clash API/多监听/TUN）、outbounds（含 urltest/手动选择 selector）、route 规则编译都在这里。
- `xray.rs` — ★ Xray 生成器（参照 v2rayN `CoreConfig/V2ray/*`）：mixed/tun inbounds + sniffing、vmess/vless(flow)/ss/trojan/socks/http/wireguard outbounds + streamSettings（tls/reality + ws/grpc/http/httpupgrade）、routing（`full:`/`domain:`/关键词/geosite:/geoip:/process 映射、balancer+observatory=kernel 自动选路）、DNS 出口分流（dns-module/direct-dns inboundTag 规则）、stats/metrics（`/debug/vars`）。无 selector 出站——主目标=选中节点 tag 或 balancer，**切节点即重启**。REALITY 仅支持 tcp/grpc 传输（ws 组合在生成期报错跳过）。用户自建远程 `.srs` 集**跳过**（Xray 不识别），内置 3 条映射为 geosite/geoip。`skip-cert-verify` 节点不输出 `allowInsecure`（Xray ≥ 26 已移除该字段，输出会导致整个配置加载失败），证书校验保持开启并记录告警。
- `dns_build.rs` — sing-box 1.12+ `dns` 对象：解析器池、统一规则集选解析器、Hosts predefined server、FakeIP。
- `write.rs` — 原子写 `active.json`（两内核共用同一文件）；custom 配置原样持久化。
- `rule_files.rs` / `dns_files.rs` — 规则/DNS 落盘为 sing-box 引用的文件。
- `custom.rs` — 自定义 sing-box 配置的检查（`inspect_singbox_config`）。
- `punycode.rs` — 域名 punycode。

### 5.5 内核管理（core/）— 双内核（sing-box / Xray）

- `kind.rs` — ★ `CoreKind` 描述符：binary 名、GitHub repo、release 资产命名（**两内核命名规则不同**：sing-box `sing-box-1.13.15-darwin-arm64.tar.gz` vs Xray `Xray-macos-arm64-v8a.zip`）、CLI 参数（version/-version、check/run -c）、版本输出解析、spawn env（Xray 设 `XRAY_LOCATION_ASSET`/`XRAY_LOCATION_CERT`）、日志前缀、协议支持集。
- `manager.rs` — 进程生命周期（`CoreKind` 参数化）：sing-box `check -c` / Xray `run -test -c` 校验 → `run -c` 启动；Windows `CREATE_NO_WINDOW`；CoreState 状态机；优雅停止；TUN 提权链路内核无关（helper 按二进制名推断 kind）。
- `download.rs` — GitHub Releases 下载/更新（按 kind 选 repo/资产/提取目标；Xray zip 额外提取 geodata）。
- `assets.rs` — Xray 资产三连：`ensure_geodata`（staged→bundled→Loyalsoldier v2ray-rules-dat 下载）、`ensure_wintun`（Windows TUN，Xray zip 不带 wintun.dll）。
- `job.rs` — Windows Job Object 绑定子进程，父进程异常退出时内核随之死亡（防端口占用残留）。
- `elevate.rs` / `macos_auth.rs` / `macos_net.rs` — TUN 提权（Windows UAC / macOS 授权）。
- `memory.rs` — 内存占用探测（Windows 用 NT 进程表 RSS）。
- `paths.rs` — 内核二进制/版本文件路径解析（resource 目录 → data 目录 staging，含 geodata/wintun 随 Xray staging）。sing-box 保持 `bin/sing-box`+`version.txt` 存量布局；Xray 用 `bin/xray`+`xray-version.txt`。

### 5.6 运行时编排与外部 API

- `runtime.rs` — `Runtime`/`ProxyStatus`（含 `core_type`）：按 `settings.core_type` 分支 config 生成 → 写盘 → core 启停 → 系统代理联动；连接视图缓存与 delta（`LiveConnectionBatch` revision 机制）。Xray 分支 `start_xray_proxy`：ensure geodata/wintun → `build_xray_config` → 就绪=进程存活+mixed port，`xray_metrics` 替代 clash_api；`build_options()` 为两生成器共享的 BuildOptions 构造器。
- `api/clash_api.rs` — Clash 兼容 API 客户端（sing-box 模式）。**HTTP 用 ureq（非 reqwest::blocking，避免嵌套 Tokio runtime panic，见文件头注释）；WS 用 tungstenite 仅握手**。
- `api/xray_metrics.rs` — Xray 模式 metrics 客户端：轮询 `/debug/vars` 汇总 `stats.outbound[*].uplink/downlink` → TrafficTotals（connections 恒 0；无逐连接 API）。
- `state.rs` `select_current_node_serialized` — sing-box 走 clash select_proxy 热切换；Xray 无 API → 持久化后返回 restart_needed，由 `rule_apply::request_restart` 重启生效；不支持的协议节点直接报错。
- `services/latency.rs` — 测速：TCP 协议直连 server:port（内核无关）；UDP 系协议（hysteria2/tuic）走 Clash delay API（Xray 模式下此类节点本就不被支持）。
- `services/import.rs` — 订阅 URL 去重键、导入文件读取。
- `srs.rs` — `.srs` 二进制规则集结构解析（LOUDS trie），供列表/计数/校验（`list_remote_rule_items` 的后端；固定用 sing-box 二进制 decompile）。
- `smart_switch.rs` / `rule_apply.rs` / `remote_rule_auto.rs` / `builtin_remote_rules.rs` — 见 5.1。smart_switch 在 Xray 模式禁用（依赖连接日志）。
- `conn_journal.rs` — 连接日志（活跃快照 + 已关闭请求历史 + 失败请求），`list_connections/list_connection_changes/list_requests/list_request_failures` 的数据源；Xray 模式降级为 metrics 轮询（仅流量）。

### 5.7 系统集成

- `proxy/windows.rs|macos.rs|stub.rs` — 系统代理设置（注册表 / networksetup），含 owned-proxy 标记与崩溃残留清理（启动时 `cleanup_stale_system_proxy`）。
- `tray.rs` — 托盘菜单 + 图标状态刷新（8 种托盘图标，`src-tauri/icons/tray/`）。
- `window_ctrl.rs` — 窗口 show/hide/destroy（托盘内存管理）、ui_mode 偏好持久化；尺寸常量与前端 `windowLayout.ts` 对应。
- `url_scheme.rs` — 注册并抢占 `clash://` `sing-box://` `singbox://` 为默认（深链一键导入）。
- `autostart.rs` — 开机启动（macOS LaunchAgent）。
- `app_log.rs` / `log_retention.rs` — 自有日志系统（trace~error 分级、panic hook、保留策略）。
- `error.rs` — `AppError`/`AppResult`。

### 5.8 commands/ 分层（前端 invoke 的直接实现）

`config.rs`（订阅 CRUD/激活/mix、`generate/preview_singbox_config` 按 core_type 分发生成器）、`core.rs`（启停/重启/capture_mode/双内核下载更新/`set_core_type` 切内核）、`connections.rs`（连接/请求/失败；`list_connection_changes` 增量协议：带 `lastOrderRevision`，纯计数更新不下发 `order_ids`）、`diagnostics.rs`、`dns.rs`（DNS+hosts）、`latency.rs`、`logs.rs`、`proxy.rs`（状态/系统代理/TUN）、`rules.rs`（规则集 CRUD/排序/远程规则，1167 行）、`subscription.rs`（导入各来源）。command 名与 `src/api.ts` 导出一一对应（snake_case）。

## 6. 前端模块详解（src/）

### 6.1 骨架

- `main.tsx` → `App.tsx`：`ThemeProvider > LocaleProvider > UiModeProvider > ImportIntentProvider > AppShell`。
- `AppShell` 按 `mode` 选 `SimpleShell` / `ProShell`；监听 `config-apply-status` 事件驱动全局 busy 与错误 banner。
- `ProShell`：`useState<NavKey>`（dashboard|config|nodes|traffic|logs|settings）+ `TopNav`；页面 `React.lazy` + `key={nav}` 强制重挂载触发进场动画。
- `UiModeContext.tsx`（`src/ui/`）— localStorage `satelite.uiMode` 先行渲染防闪烁；切模式先调 `set_ui_mode_pref` 让 Rust 调窗口尺寸再换 shell。`UiModeMenu.tsx` — 工具栏 "⋯" 菜单（模式切换/重启内核/复制代理环境变量）。

### 6.2 桥接层 ★

- `api.ts` — 全部 `invoke()` 封装。要点：
  - `updateSettings` 是 **60ms 批量合并写入器**；
  - `peekSettings/peekProxyStatus + keepSettings/keepProxy` 模块级快照，供页面重挂载时种子状态防闪默认值；
  - 生命周期类调用（start/stop/restart/capture/outbound）包 `trackCoreBusy()` 驱动导航栏 spinner。
- Tauri 事件消费：`config-apply-status`（App.tsx）、`deep-link-urls`（ImportIntentContext）、`core-download-progress`（SettingsPage）、`remote-rule-set-status` 与 `rule-set-apply-status`（RulesPage）。
- `types.ts` — 与 Rust `domain/*` 对应的手写类型（注意 `ProxyStatus`、`AppSettings`、`ManualNodeDraft` 46 字段等需两边同步）。

### 6.3 页面（pages/）

| 页面 | 要点 |
|---|---|
| `DashboardPage` (1399 行) | 启停/重启、capture/出站模式快控、节点选择、配置预览弹窗、60 样本迷你图、LAN IP、版本 |
| `ConfigPage` (831) | 订阅卡片（流量配额条）、排他选择/Mix、`AddConfigModal`、深链预填（`useImportIntent`） |
| `NodesPage` (464) | 列表/网格（`useVirtualRange`×2）、搜索排序测速、改名、custom 配置节点；切节点 `waitForCoreRestart` |
| `TrafficPage` (45) | 三 tab 容器：实时连接 / 请求历史 / 失败请求 |
| `ConnectionsPage` (215) | 1.5s revision-delta 增量轮询（`list_connection_changes` + `applyConnectionChanges`） |
| `RequestsPage` (258) / `FailuresPage` (510) | 已关闭请求/失败请求日志；Failures 可一键生成封锁规则集 |
| `LogsPage` (207) | 应用日志查看（1.2s 增量，级别过滤+搜索） |
| `SettingsPage` (1456) | 6 tab：app/ports/rules/dns/hosts/core；内嵌 Rules/Dns/Hosts 页；内核下载进度事件、更新检查、诊断、托盘图标选择、赞助二维码（`DecryptReveal`） |
| `RulesPage` (2145) | ★ 最大页面：规则集侧栏+编辑器、本地/远程集、策略/DNS 策略、route.final、拖拽排序、远程规则项浏览 |
| `DnsPage` (329) / `HostsPage` (463) | DNS/Hosts 管理，通常内嵌于 Settings |

### 6.4 简洁模式（ui/simple/）

`SimpleShell`（4 tab：connect/servers/traffic/settings）+ 各页。复用玻璃设计语言与 `AddConfigModal`；`SimpleTrafficSpark` 为 SVG 迷你流量图。新增面向普通用户的轻量入口时改这里。

### 6.5 组件与 hooks

- 设计系统：`GlassButton`、`GlassSeg`（区分用户点击与状态重绘才做动画）、`GlassSwitch(+Control)`、`SolidSelect`（**自绘下拉：macOS WKWebView 原生 select 无法主题化**，SolidSelect.tsx:26 注释）。
- 首页视觉：`HeroVisual`（按 `heroStyle` 分发）→ `ParticleSphere`（three.js，lazy）/ `FaceMark`（Canvas2D 笑脸）/ 经典轨道。
- 弹窗：`AddConfigModal`（url/file/paste/手动节点/sing-box 五种来源）、`EditLocalNodesModal`、`NodeDraftFields`（16 协议条件字段表单，与 `ManualNodeDraft` 对应）、`AccentColorPickerModal`（自定义主题色取色器；`theme/accents.ts` 支持 `#rrggbb` 自定义 accent，Rust `update_settings` 同步放行）、`DecryptReveal`。
- hooks：
  - `useVisibleInterval` — **通用轮询原语**：页面隐藏暂停、回调不重叠、可见即重发；
  - `useVirtualRange` — 基于 `.main` 滚动容器的列表虚拟化（支持网格 itemsPerRow）；
  - `useRulesetDragSort` — 手写指针拖拽排序（Tauri WebView 里 HTML5 DnD 不可靠，见文件头注释）：5px 阈值、LERP 跟随克隆、FLIP 动画、边缘自动滚动、Esc 中止；
  - `useCaptureModeSwitch` — 乐观切换 + 单飞排空队列（防内核并发切换报错）。

### 6.6 i18n / 主题 / 其他工具模块

- `i18n/messages.ts` — `en`（630 键，`as const`）+ `zh: Record<MessageKey, string>`。**加文案必须两边同加，否则 TS 编译错**。键前缀：`common./nav./simple./dashboard./nodes./config./traffic./conn./logs./settings./rules./dns./hosts./failures.`；`translate()` 支持 `{n}` 插值。
- `theme/` — `ThemeId = "aerospace"(深,默认) | "day"`；`accents.ts` 6 个主题色，由一个基色派生整个 `--primary*` 变量族（Rec.709 亮度决定 `--on-primary`）。语义色 `--success*` 为固定绿（App.css tokens），**不随主题色**（ok/直连/测速良好语义稳定）；自定义 `#rrggbb` accent 在 `applyAccentToDom` 应用时按主题做亮度钳制（深色提亮 ≥0.5 / 浅色加深 ≤0.6）保证文字对比度，存储仍保留原始 hex。另有独立背景光晕色 `glow_color`（`"accent"`=跟随主题色 / 预设 id / `#rrggbb`），`applyGlowToDom` 下发 `--glow-rgb`（原始色，驱动 `--hero-glow`）与 `--glow-deep-rgb`（按感知亮度归一化的深色变体，驱动 app-shell 大气层，防止亮色光晕把暗色主题洗亮）。
- 独立模块：`customNodes.ts`（custom 节点客户端侧过滤/排序/分页镜像）、`subscriptionUrl.ts`（URL 规范化去重）、`deepLink.ts`（深链解析→ImportPrefill）、`coreBusy.ts`（全局 busy 深度计数 + `waitForCoreRestart`）、`connectionChanges.ts`（delta 合并纯函数）、`trafficFilter.ts`（all/direct/proxy 分类）、`windowLayout.ts`（窗口尺寸/模式）。
- `App.css` — 单文件 ~7.6k 行，按 `/* —— 段落 —— */` 横幅分节（tokens → shell → topnav → page → nodes → …）；玻璃材质 = 半透明 rgba + `backdrop-filter` + 左上光源 `::after`；专业窗口固定 960px 宽（网格断点据此调）。

## 7. 常见修改场景 → 去哪里改

| 需求 | 位置 |
|---|---|
| 新增设置项 | `domain/settings.rs`（`AppSettings`）→ `storage/store.rs`（迁移如需）→ `config/builder.rs` **和/或 `config/xray.rs`**（生成如需，双内核都要考虑）→ `src/types.ts`（`AppSettings`）→ 页面 UI + `i18n/messages.ts` 双语 |
| 新增 command | `src-tauri/src/commands/<域>.rs` → `commands/mod.rs` re-export → `lib.rs` `generate_handler![]` 注册 → `src/api.ts` 加封装 |
| 新增订阅格式/协议解析 | `src-tauri/src/subscription/`（clash/singbox/uri/manual）+ `domain/node.rs`（新协议记得看 `CoreKind::supports` 是否要放行 Xray） |
| 改 sing-box 配置生成 | `config/builder.rs`（路由/inbound/outbound）、`config/dns_build.rs`（DNS） |
| 改 Xray 配置生成 | `config/xray.rs`（改动后用 `xray run -test -c` 手工验证，失败退出码 23） |
| 改规则集逻辑 | `domain/rule.rs`（模型）+ `config/builder.rs`（sing-box 编译）+ `config/xray.rs`（Xray 映射）+ `commands/rules.rs` + `src/pages/RulesPage.tsx` |
| 改内核启动参数/生命周期 | `core/manager.rs` + `core/kind.rs`（kind 相关差异集中在 kind.rs） |
| 加文案 | `src/i18n/messages.ts` 的 `en` 和 `zh` **都要加** |
| 加页面 | `src/pages/` + `App.tsx` lazy 导入 + `NavKey`（types.ts）+ `TopNav` + i18n `nav.*` |
| 改样式 | `src/App.css` 对应段落；新主题色变体在 `theme/accents.ts` |
| 加托盘功能 | `src-tauri/src/tray.rs` |
| 改测速 | `services/latency.rs` + `src/api.ts` testNodesLatency |
| 改内核下载/资产 | `core/download.rs` + `core/assets.rs` + `scripts/fetch-bundled-*-<平台>` 脚本 + `tauri.*.conf.json` resources 四处联动 |
| 重大架构 / 模块 / 流程变动 | **同步更新本文档对应章节**（规则见 §0） |

## 8. 构建细节与产物

- **版本号**：`package.json`（1.0.9）是唯一真源，`tauri.conf.json` 引用它；`Cargo.toml`（1.0.4）落后且不自动同步——发版时手动检查三处。
- **产物路径**：DMG → `src-tauri/target/<aarch64|x86_64>-apple-darwin/release/bundle/dmg/`；Windows → `src-tauri/target/release/bundle/nsis/`（或 `.../msi/`）。
- **Rust 测试布局**：集成测试 `src-tauri/tests/parse_subscription.rs`（fixtures 在 `tests/fixtures/`：clash yaml ×2、singbox json ×1）；`download_core_live.rs` 为 `#[ignore]` 真网测试；单测散落各文件 `#[cfg(test)]`。
- **换行符**：`.gitattributes` 规定源码 eol=lf、`.ps1/.bat/.cmd` 为 CRLF。
- **内核版本**：macOS 预取脚本默认 sing-box v1.13.18，Windows v1.13.15，两者独立演进，升级时分别改脚本；Xray 各平台统一 v26.3.27（`scripts/fetch-bundled-xray-*` + `core/kind.rs::fallback_version` 两处同步）。

## 9. 约定与坑（agent 必读）

1. **Clash API 客户端禁用 `reqwest::blocking`** — 嵌套 Tokio runtime 会在 Tauri async worker panic；用 `ureq`（`api/clash_api.rs` 文件头有说明）。reqwest 仅用于异步下载内核。
2. **`resources/bin/**/sing-box*`、`xray*`、`*.dat`、`wintun.dll`、`libcronet.dll`、`resources/rule-sets/*.srs` 不入库** — 本地没有属正常，dev 首次运行自动下载。
3. **`BUILTIN_REMOTE_RULE_SETS`（`domain/rule.rs`）与 `scripts/fetch-bundled-rule-sets.sh` 必须同步**；内置 3 条的 Xray geosite 映射在 `config/xray.rs`（`builtin_remote_xray_rule` + DNS 分类处），改 id 时三处联动。
4. **i18n 双语强约束** — `messages.ts` 中 `zh` 的类型是 `Record<MessageKey, string>`，漏键编译失败。
5. **前端↔后端类型手工同步** — `src/types.ts` 与 `domain/*` 无代码生成；改 Rust 序列化结构记得同步 TS（部分 invoke 同时发 camelCase+snake_case 参数以兼容，见 `api.ts`）。
6. **单窗口** — 无多窗口 API 用法；窗口尺寸/可调性由模式决定（pro 960×720 固定 / simple 420×720 可调 320–420 宽）。
7. **平台差异** — 系统代理 `proxy/windows.rs|macos.rs`（Linux 用 stub）、TUN 提权 `core/elevate.rs`（Win）与 `core/macos_auth.rs`、进程绑定 `core/job.rs`（仅 Win）。改平台行为时注意 cfg 分支。
8. **HTML5 拖拽在 Tauri WebView 不可靠** — 排序一律用 `useRulesetDragSort` 模式（指针事件手写）。
9. **页面切换 = 重挂载**（`key={nav}`）— 页面自身状态不跨切换保留；跨页面共享靠 `api.ts` 模块级快照（peek/keep）。
10. **规则变更应用是防抖+串行**（`rule_apply.rs` 500ms 合并）— UI 事件 `rule-set-apply-status` 回报结果，不要假设保存即重启完成。
11. **store.json 解析失败会拒启**（防覆盖用户新 schema 数据）；未知字段保留在 `retained_*` 写回。改存储结构时保持向后兼容 + `schema_version` 迁移。
12. **窗口关闭默认进托盘**；真正退出需 `exit_allowed`（`state.is_exit_allowed()`），退出时 `shutdown_runtime()` 停内核清代理。
13. **双内核配置生成相互独立** — `config/builder.rs`（sing-box）与 `config/xray.rs` 不共享生成代码，只共享 domain 模型与 `BuildOptions`；改路由/协议/DNS 语义时**两边都要改**并各跑单测。
14. **Xray 无 Clash API** — 无逐连接数据/热切节点/delay API：切节点与规则变更=重启进程（`select_current_node_serialized` 返回 restart_needed）；连接三页面在 Xray 下为空态；smart_switch 禁用。流量统计靠 metrics `/debug/vars`（`api/xray_metrics.rs`）。
15. **Xray 资产依赖** — `geosite:`/`geoip:`（含 `geoip:private`）需要 geosite.dat/geoip.dat（bundled 或运行时下载，`core/assets.rs::ensure_geodata`）；Windows TUN 需要 wintun.dll（Xray zip 不带）。缺资产时 Xray 启动会失败，报错要可读。
16. **`.srs` 规则集是 sing-box 专有** — Xray 生成器跳过用户自建远程 `.srs` 集（内置 3 条走 geosite 映射）；`srs.rs` decompile 固定用 sing-box 二进制。
