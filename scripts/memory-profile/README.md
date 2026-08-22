# WebView2 内存剖析工具

测量 Satelite 的 WebView2 进程树内存 + 页面级 JS 堆，用于内存回归基线对比。
完整方法论与基线数据见 `docs/webview2-memory-optimization-plan.md`。

## 用法

1. 带远程调试端口启动 dev（端口 9222）：

   ```bash
   # Git Bash / macOS / Linux
   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" pnpm tauri dev
   ```

2. 进程树内存（按父进程链过滤出属于 satelite-proxy 的 msedgewebview2 进程）：

   ```powershell
   pwsh scripts/memory-profile/proc_mem.ps1 -RootName satelite
   ```

3. 页面级采样（强制 GC 后取 `performance.memory` + Performance.getMetrics）：

   ```bash
   node scripts/memory-profile/cdp.mjs snap baseline          # GC 后一次采样
   node scripts/memory-profile/cdp.mjs snap raw --nogc        # 不 GC
   node scripts/memory-profile/cdp.mjs soak dash 360 60       # 浸泡：raw/gc 交替
   node scripts/memory-profile/cdp.mjs eval "<js>"            # 任意表达式
   node scripts/memory-profile/cdp.mjs rclick ".topnav-item" 0  # 直调 React onClick 驱动导航
   node scripts/memory-profile/cdp.mjs shot out.png           # 截图
   ```

## 注意

- WebView2 窗口非前台时 Input 事件注入不可用，`rclick`（直调 `__reactProps$.onClick`）是可靠驱动方式。
- GPU 进程的 PrivatePageCount 含弹性缓存（闲置会被 Chromium 逐出），单点数值只看趋势/峰值。
- 测完 Ctrl+C 结束 dev；调试端口随进程关闭。
