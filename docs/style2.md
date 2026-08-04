# Proxy Control Center UI Design System

## Aerospace Telemetry Style Design Specification

**项目定位**

一个基于 sing-box 内核的高级代理客户端。

设计目标：

> 将代理软件从「工具型配置面板」升级为「个人网络控制中心」。

视觉方向：

**Neo Aerospace Network Operations Console**

融合：

* 航天任务控制台
* 网络运维 NOC
* Cyberpunk 工业终端
* 高级开发者工具

---

# 1. Brand Direction

## 产品气质

关键词：

| 属性   | 描述              |
| ---- | --------------- |
| 专业   | 像工程设备，而不是普通 App |
| 精密   | 数据驱动、状态透明       |
| Geek | 面向高级用户          |
| 快速   | 强调 sing-box 性能  |
| 可信   | 显示真实网络状态        |

避免：

* Clash 类软件常见彩色卡片
* 大圆角 SaaS 风格
* 游戏化 UI
* 过多动画

---

# 2. Overall Layout

## 主窗口结构

```
┌─────────────────────────────────────┐
│ STATUS BAR                          │
│ CORE  NETWORK  DNS  ROUTING        │
├──────────┬──────────────────────────┤
│          │                          │
│ SIDEBAR  │ MAIN PANEL               │
│          │                          │
│          │                          │
│          │                          │
├──────────┴──────────────────────────┤
│ SYSTEM TELEMETRY                    │
└─────────────────────────────────────┘
```

---

# 3. Sidebar Design

## 定位

类似：

* 飞船控制面板
* IDE Explorer
* Linux 工具栏

宽度：

```
220px
```

---

## Logo 区域

```
┌───────────────┐

     ◈
  NEXUS CORE

 sing-box engine

└───────────────┘
```

Logo 不做品牌插画。

使用：

* 几何符号
* 单色线框

---

# Sidebar Menu

```
NETWORK

 ◉ Dashboard

 ◇ Nodes

 ◎ Connections

 ◌ Traffic


CONTROL

 ◇ Routing

 ◇ DNS

 ◇ Rules


SYSTEM

 ◇ Core

 ◇ Logs

 ◇ Settings
```

---

# Active 状态

当前页面：

```
│
│ ◉ Dashboard
│
```

效果：

* 左侧 3px 发光线
* 青绿色文字
* 微弱背景

不要使用：

* 大面积按钮
* 圆角胶囊

---

# 4. Color System

## Background

Primary:

```
#08090A
```

Panel:

```
#111315
```

Border:

```
#272B2D
```

Grid:

```
#1B2022
```

---

## Accent

### Sing-box Green

```
#61E6C1
```

用途：

* Connected
* Active
* Running
* Speed

---

## Warning

```
#E8B84B
```

用途：

* Delay
* DNS fallback
* Expiring subscription

---

## Error

```
#E85C5C
```

---

# 5. Typography

## Font

推荐：

Primary:

```
JetBrains Mono
```

Alternative:

```
IBM Plex Mono
Space Mono
```

---

## Text Style

### Section Title

```
NETWORK STATUS
```

12px

uppercase

letter spacing:

```
1.8px
```

---

### Data Number

例如：

```
1.28 GB/s
```

48px

---

### Metadata

```
TCP
443
TLS
```

11px

---

# 6. Dashboard 首页设计

目标：

打开软件 3 秒知道：

* 有没有代理
* 当前节点
* 网络状态
* DNS状态
* 流量

---

# Dashboard Layout

```
┌───────────────────────────┐
│ CORE STATUS               │
│                           │
│ RUNNING       12:34:22    │
└───────────────────────────┘


┌──────────┬───────────────┐
│ NODE     │ DNS           │
│ Tokyo    │ Fake-IP       │
└──────────┴───────────────┘


┌───────────────────────────┐
│ NETWORK TELEMETRY         │
│                           │
│    /\                     │
│   /  \____               │
│                           │
│ ↓ 120MB/s ↑ 30MB/s       │
└───────────────────────────┘
```

---

# 7. Core Status Card

类似航天状态模块。

```
CORE STATUS


● OPERATIONAL


sing-box

v1.11.0


UPTIME

03:21:55
```

状态：

绿色：

```
OPERATIONAL
```

异常：

```
DEGRADED
```

---

# 8. Node Card

替代传统 Clash 节点列表。

```
ACTIVE RELAY


TOKYO-01


LATENCY

38ms


PROTOCOL

Hysteria2


NETWORK

IPv6
```

---

# 9. Traffic Visualization

不要做普通折线。

采用：

## Telemetry Graph

特点：

* 黑底
* 网格
* 单线
* 无 tooltip

显示：

```
DOWNLOAD

128 MB/s


UPLOAD

24 MB/s
```

---

# 10. DNS Dashboard

核心卖点页面。

```
DNS CONTROL


MODE

FAKE-IP


RESOLVER


223.5.5.5


RULE OVERRIDE


CN DOMAIN

SYSTEM DNS


PROXY DOMAIN

REMOTE DNS
```

---

# 11. Routing Visualization

类似网络拓扑。

```
REQUEST


github.com


       |
       |

RULE ENGINE


       |
 ┌─────┴─────┐

DIRECT     PROXY
```

---

# 12. Node Explorer

不要传统列表。

采用：

```
NODE MATRIX
```

例如：

```
┌───────────────┐
│ Tokyo JP      │
│               │
│ 38ms          │
│ 98% health    │
└───────────────┘


┌───────────────┐
│ Singapore SG  │
│               │
│ 52ms          │
└───────────────┘
```

---

# 13. Logs Design

类似飞船日志。

```
SYSTEM LOG


08:32:10

DNS_REROUTE


Domain:

api.github.com


Resolver changed:

SYSTEM → REMOTE
```

---

# 14. Animation Rules

允许：

* 状态扫描线
* graph 动态
* loading

禁止：

* 页面切换动画
* 大量粒子
* 炫光

---

# 15. Component Library

## Button

```
[ ENABLE ]
```

风格：

* 方形
* 1px border
* hover 发光

---

## Switch

类似硬件开关：

```
POWER

[ ●──── ]
```

---

## Badge

```
TCP
TLS
UDP
```

矩形标签。

---

# 16. Icon System

推荐：

Lucide Icons

修改：

* stroke 1.2px
* 单色

图标：

```
◎
◇
△
⌁
```

---

# 17. Empty State

不要插画。

例如：

```
NO ACTIVE CONNECTION


Waiting for network activity...
```

---

# 18. 软件整体氛围

最终感觉：

```
打开软件

↓

像进入自己的网络控制舱

↓

看到：

Core
DNS
Routing
Traffic
Nodes

↓

所有状态透明
```

---

# 19. 技术实现建议（Tauri 2）

UI：

* React + Tailwind
* CSS variables 管理主题
* Canvas/WebGL 绘制 telemetry

字体：

```
JetBrains Mono
IBM Plex Mono
```

图表：

* 自绘 SVG
* Canvas

避免：

* Ant Design
* Material UI
* Bootstrap

因为这些组件库会破坏工业控制台感。

---

# Design Keyword

最终设计标签：

```
Aerospace Network Console

+
Cyber Industrial Terminal

+
High Density Telemetry UI

+
Developer First Proxy Client
```

这个方向会比 Clash/Mihomo 生态里的大多数 GUI 更像一个「专业网络操作系统」。

