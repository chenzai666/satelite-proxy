# WebView2 内存优化计划

> 基线版本：v1.0.8 工作区（2026-08-22）· WebView2 Runtime **151.0.4129.93** · Tauri 2.11.5 / wry 0.55.1 · Windows 10 (26200) x64
> 本文档全部数据来自真实进程测量 + CDP（Chrome DevTools Protocol）堆采样，非估算。

## 0. TL;DR

| 结论 | 数据 |
|---|---|
| **JS 堆不是问题** | 全程 8 → 19MB（GC 后 8-12MB），业务数据未失控 |
| **GPU 进程是最大且最弹性的消耗者** | 私有内存 137MB（冷启）→ **235~272MB**（活跃使用），静置可回落到 ~165MB |
| renderer 进程是 Blink/V8 基线主导 | 私有 70~85MB，与页面数据量弱相关 |
| 固定开销 | browser 42MB + utility×2 23MB + crashpad 3MB ≈ **68MB**（不可优化） |
| 全树合计 | 私有提交 **275MB（冷启）→ 421MB（活跃峰值）**；WS 合计 432~509MB |
| 最大单项优化 | 托盘隐藏时销毁 WebView（`unloadUiOnTray` 已实现）→ 托盘态全树归零 |
| 前台最大优化 | Glass 控件家族去 backdrop-filter → GPU 预期 -30~80MB |

---

## 1. 测量方法

### 1.1 环境

- `pnpm tauri dev`（debug 构建），携带用户真实 store（含订阅/节点/规则），sing-box 运行中，Chrome 经 127.0.0.1:2080 走真实代理流量。
- WebView2 以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"` 启动，开启 CDP。

### 1.2 工具

- **进程级**：PowerShell 按「进程父子链 + `--webview-exe-name`」过滤出属于 satelite-proxy.exe 的 WebView2 进程树，分别记录 WorkingSet 与 PrivatePageCount（`%TEMP%\satelite-mem\proc_mem.ps1`）。
- **页面级**：CDP `HeapProfiler.collectGarbage` 强制 GC 后取 `performance.memory` + `Performance.getMetrics`（DOM 节点数、事件监听器数等）（`%TEMP%\satelite-mem\cdp.mjs`）。
- 页面驱动：因 WebView2 输入管线在窗口非前台时不接受注入事件，采用直调 `__reactProps$.onClick` 的方式遍历导航（脚本已内置）。

### 1.3 测试序列（层序）

L1 冷启动基线（+30s，概览页 hero 活跃）→ L2 页面遍历（节点/配置/流量三子页/日志/设置六 tab，每页停 7-25s）→ L3 回访泄漏检测 → L4 静置浸泡（并行进程采样）→ L5 用户真实活跃会话采样。

## 2. 真实测量数据

### 2.1 进程树（Satelite 专属 WebView2，共 6 进程）

| 场景 | WS 合计 | 私有合计 | GPU priv | Renderer priv | Browser priv |
|---|---|---|---|---|---|
| 长跑旧会话（活跃使用数十分钟） | 509MB | 380MB | 212MB | 101MB | 42MB |
| 冷启动 +30s（概览 + three.js hero） | 432MB | 275MB | 137MB | 70MB | 42MB |
| 页面遍历后（活跃） | 489MB | 381MB | **235MB** | 78MB | 42MB |
| 静置 5min（无操作） | 490MB | 316~347MB | 164~198MB（回落） | 80~85MB | 42MB |
| 用户当前活跃会话（复测） | 480MB | **421MB** | **272MB** | 81MB | 42MB |

要点：

1. **GPU 进程私有内存随「见过的界面多样度」爬升**（137→235→272MB），**闲置数分钟后会被
   Chromium 主动逐出回落**（实测 235→164MB；同会话注入样式 A/B 中 blur ON/OFF 差异被逐出
   噪声淹没，说明该指标只能测峰值与趋势，测不出层级别增量）。blur 的成本是架构性的
   （每层独立 backdrop 采样 pass），体现在合成开销与峰值上。这是用户感知「200+MB」的最主要成分。
2. WS 合计跨进程重复计算共享页，**私有合计（275~421MB）才是真实足迹**。
3. 应用自身 Rust 进程仅 42~62MB，sing-box ~47MB，均与本计划无关。

### 2.2 页面级（CDP，GC 后）

| 页面 | JS heap used | DOM 节点 | 监听器 | 备注 |
|---|---|---|---|---|
| 概览（冷启，hero canvas） | 12.1MB | 175 | 196 | three.js chunk 已加载 |
| 节点 | 15.3MB | 1363 | 297 | 虚拟化生效，DOM 有界 |
| 配置 | 14.1MB | 227 | 191 | |
| 流量·连接中（跑 60s 真实连接） | 16.2MB | 320 | 241 | 当前连接数少，未显形 |
| 流量·已关闭 / 失败 | 15.9MB | 125-141 | 171-179 | |
| 日志 | 15.8MB | 89 | 169 | |
| 设置·应用（8 图标） | 17.1MB | 167 | 308 | glass 开关/分段最密集页 |
| 设置·规则（RulesPage 全量挂载） | 16.6MB | ~555 | 254 | |
| 浸泡尾段（用户停在设置页） | 19.0MB | 125 | — | 15min 后仍 <20MB |

**回访检测**：页面切换（`key={nav}` 重挂载）后 DOM/监听器回落到基线量级，未观察到跨页累积；JS 堆稳态 <20MB。**无恶性泄漏**。

## 3. 内存构成结论（按占用排序）

1. **GPU 进程（137~272MB priv）**——三股力量：
   - `backdrop-filter` 模糊层：全 css 22 处使用，其中 `.glass-seg/.glass-btn/.glass-switch-track`（`App.css:5449/5548/5689`）作用在**同屏几十个重复控件**上（设置页 7+ 开关 × 2 层/个 + 3 分段 + 胶囊），每个模糊层需独立 backdrop 纹理 + blur 半径×2 的扩边；
   - three.js WebGL 上下文：460×460 CSS px × DPR2 的 framebuffer 常驻；`ParticleSphere.tsx` dispose 完整但**缺 `forceContextLoss()`**（`ParticleSphere.tsx:746`），上下文归还依赖 canvas GC；heroStyle 频繁切换会短期堆积；
   - 常驻合成动画：经典轨道 5 条 infinite spin（`App.css:4955-4972`）+ hero 光晕 blur，使 GPU 合成永不空闲。
2. **Renderer 进程（70~85MB priv）**——Blink/V8/已加载代码基线为主（JS 堆仅 8~19MB，DOM ≤1363）。可压缩项：死 CSS（`.simple-card`/`.simple-glass-bar` blur、`pulse-dot` keyframes）、three.js chunk 常驻（~1.5MB+，切走 hero 不卸载）、800 行×3 张无虚拟化长表（当前数据少未显形，TUN/高连接场景会放大到万级 DOM）。
3. **固定开销 68MB**（browser 42 + utility 23 + crashpad 3）——不可优化，构成本应用 WebView2 的理论下限。
4. **数据层风险（当前未显形）**：`ConnectionsPage` rows 无上限且每 1.5s 全量重建数组+Map（`connectionChanges.ts:10-17`，Rust 侧 `order_ids` 每次发全量 N 个 id，`runtime.rs:503-507`）；后台 100ms 全量快照使 revision 持续变化，`unchanged` 快路径难命中。

## 4. 优化项清单

### P0 · GPU 进程减负（预期合计 -40~110MB 活跃态）

**P0-1 Glass 控件家族去 backdrop-filter**（预期 GPU -30~80MB）
- 证据：`App.css:5449`（.glass-seg blur18）、`App.css:5548`（.glass-btn blur18）、`App.css:5679`（.glass-switch-track blur14）；设置页同屏 7+ 开关 → 14+ 独立模糊层。
- 改法：重复小控件改为**静态半透明底 + 1px inset 高光**（视觉近似、零 backdrop 采样）；或每张卡片只保留一个共享模糊底板层，控件继承。`.topnav`（blur20，唯一容器）和 `.card`（每页个位数）可保留。
- 风险：视觉微差；需截屏对比 6 主题色 × 深/浅主题。
- 验证：GPU priv 活跃峰值对比（预期 272 → 200MB 以内）。

**P0-2 ParticleSphere 补 `forceContextLoss()`**（**❌ 2026-08-22 实验否决，勿做**）
- 实测：加 `forceContextLoss()` 后，连续 6 次快速切换 heroStyle（粒子↔经典↔笑脸）会触发
  WebView2 (151.0.4129.93) 渲染进程的 WebGL 上下文创建失败窗口——随后 `new THREE.WebGLRenderer`
  抛异常，hero 永久落到 fallback（组件把失败缓存在 state）；去掉 forceContextLoss 后同样操作
  完全正常。上下文耗尽（~16 个）只在用户极频繁切换 heroStyle 时才可能发生，属可接受场景。
- 保留原状：`renderer.dispose()` 即可。

**P0-3 hero 动画空闲治理**
- FaceMark 每帧 3 次分配（`FaceMark.tsx:302/327/292`：Path2D/颜色对象/模板字符串）→ 按 mood 缓存 Path2D、颜色原地复用；
- ParticleSphere 已零分配（优秀）；补 `visibilitychange` 显式停 rAF（当前依赖浏览器节流）。
- 收益：降低 GC churn 与合成活跃度（间接稳住 GPU 水位）。

### P1 · Renderer 与数据层防御（预期 -5~15MB + 防长期膨胀）

**P1-1 三张 800 行表虚拟化**：Requests/Failures/Logs（`RequestsPage.tsx:184`、`FailuresPage.tsx:311`、`LogsPage.tsx:187`）复用 `useVirtualRange`（节点页已验证方案）。当前用户数据少未显形；订阅大/日志多/TUN 场景 DOM 可达万级。
**P1-2 连接表设上限**：`ConnectionsPage.tsx:34-62` rows 无上限；按活跃连接数 cap（如 1000）+ 提示。Rust 侧 `order_ids` 改为仅在顺序变化时下发（`runtime.rs:503-507`）。
**P1-3 Dashboard 配置预览滞留**：`DashboardPage.tsx:183/483-494` 关闭弹窗时 `setResult(null)` 释放数百 KB~MB 级字符串。
**P1-4 tauri listen() 竞态**：`RulesPage.tsx:264-322`、`SettingsPage.tsx:301-308` unlisten 可能为 undefined——加 `disposed` 标志，resolve 时若已卸载立即调用 dispose。设置页内嵌 RulesPage 每次切 tab 都会走此路径。
**P1-5 小项**：FailuresPage 行内 `rowHost/extractDomainSuffix` 提到 useMemo/预计算（`FailuresPage.tsx:312-313`）；死 CSS 清理（`.simple-card`/`.simple-glass-bar`/`pulse-dot`）。

### P2 · 结构性手段（唯一能「真正降总量」的开关）

**P2-1 `unloadUiOnTray` 默认开启 + 首次托盘提示**：销毁 WebView 后全树 6 进程退出，托盘态 WebView2 占用 **归零**（-275~421MB）。已实现（`window_ctrl.rs`），只需改默认值/引导。建议同时保留托盘菜单「显示界面」重建路径的秒级体验（首帧骨架已就绪）。
**P2-2 托盘常驻节流**：窗口隐藏时 conn_journal 已降至 400ms（`conn_journal.rs:16-20`），可再降至 1s 或纯事件驱动（WS 已有）。
**P2-3 可选实验**：`additionalBrowserArguments`（tauri.conf `windows[].additionalBrowserArguments`，wry 0.55.1 已支持）尝试 `--force-low-power-gpu` 等需实测；**不建议**盲目加开关参数。注：当前栈无 `clear_all_blink_caches` API，不可用。

### P3 · 监控与回归

**P3-1 内置内存仪表**：诊断页/概览展示 `performance.memory.usedJSHeapSize` + Rust 侧 webview2 进程树 RSS（复用本次 proc_mem 逻辑）——让「内存回归」可见。
**P3-2 测量脚本固化**：将 `%TEMP%\satelite-mem` 的 `cdp.mjs`/`proc_mem.ps1` 收进 `scripts/memory-profile/`，README 注明启动方式（env var + 9222）。

## 5. 预期收益汇总

| 措施 | 前台活跃态 | 托盘态 | 成本 |
|---|---|---|---|
| P0-1 glass 去 blur | **-30~80MB (GPU)** | — | 视觉调优 |
| P0-2/0-3 hero 治理 | -10~30MB (GPU) | — | 一行+小改 |
| P1 全部 | -5~15MB (renderer) + 防膨胀 | — | 中 |
| P2-1 托盘销毁默认开 | — | **≈-300~420MB** | 改默认值 |
| **合计** | 421 → **~300-350MB**；静置 ~250MB | **≈0（仅剩 Rust+内核）** | |

绝对下限说明：Chromium 六进程固定开销 ~68MB + 应用 DOM/JS 基线，**前台状态下任何优化都无法低于 ~200MB 私有**；「200+MB」中有相当比例是 WebView2 平台固有成本。真正把数字打下去的是托盘态销毁（P2-1）。

## 6. 验证流程（发版前回归）

1. 冷启 +30s / 页面遍历 / 静置 5min 三点采样（proc_mem.ps1）；
2. heroStyle 三样式各切换 10 次 → GPU priv 无阶梯；
3. TUN 模式跑 30min 高连接（或 `curl -x 127.0.0.1:2080` 并发 200 连接）→ ConnectionsPage DOM/heap 有界；
4. 托盘隐藏 10min → msedgewebview2 进程组归零。

## 7. 测量限制声明

- debug 构建（release 前端产物未测，V8 编译缓存/代码量略不同）；
- 浸泡段被真实用户操作打断（样本已按 `nav` 标签注明页面），静置数据来自并行进程采样；
- WS 与私有内存双口径已区分；长跑样本为用户此前会话，无法追溯页面路径。

## 8. 实施记录（perf/webview2-memory 分支，2026-08-22）

| 项 | 状态 | 备注 |
|---|---|---|
| P0-1 glass 控件去 blur | ✅ 已实施 | `.glass-seg/.glass-btn/.glass-switch-track` 改实色（#232a37/#262d3a，与 modal opaque 方案同族）；计算样式验证 backdropFilter=none；设置页截图人工+视觉模型复核无缺陷；浅色主题走原有 day 覆盖 |
| P0-2 forceContextLoss | ❌ 实验否决 | 见 §4 P0-2，反而触发 WebView2 WebGL 失败窗口 |
| P0-3 FaceMark 稳态零分配 | ✅ 已实施 | 补间完成后直接引用 colorTarget，稳态不再每帧 new {r,g,b,a} |
| P1-3 预览释放 | ✅ 已实施 | closePreview() 同时 setResult(null) |
| P1-4 listen() 竞态 | ✅ 已实施 | RulesPage×2 + SettingsPage，disposed 标志模式 |
| P1-5 死 CSS + FailuresPage memo | ✅ 已实施 | 删 .simple-card/.simple-glass-bar/.simple-info-*/.simple-kv-*/pulse-dot（TSX 零引用）；行级 host/suffix 移入 useMemo |
| P1-1/P1-2 表格虚拟化与连接上限 | ⏳ 未做 | 改动面大，留待下一批 |
| P2-1 unloadUiOnTray 默认开 | ⏳ 未做 | 设置项已有（「低内存模式」），是否默认开待产品决策 |
