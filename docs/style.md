# Satelite UI 风格设计方案

## 1. 产品定位

**Satelite = Developer-grade sing-box Controller**

定位不要做成：

* ❌ 企业网络管理后台
* ❌ 普通 VPN 客户端
* ❌ Clash 面板复刻

应该接近：

* Ghostty（终端气质）
* Raycast（效率工具）
* Proxyman（网络可视化）
* OrbStack（工程工具）
* Linear（极简信息密度）

关键词：

> Dark · Dense · Native · Technical · Fast

---

# 2. 整体视觉方向

## Theme

默认：

```
Dark Mode First
```

不是黑色电竞风，而是开发工具风。

参考：

```
Background
#0D1117

Panel
#161B22

Border
#30363D

Primary Text
#E6EDF3

Secondary
#8B949E

Accent
#58A6FF

Success
#3FB950

Warning
#D29922

Danger
#F85149
```

---

# 3. Layout

## 主窗口

推荐：

```
┌─────────────────────────────────────┐
│ macOS Title Bar                      │
├──────────┬──────────────────────────┤
│          │                          │
│ Sidebar  │ Main                     │
│          │                          │
│ 220px    │                          │
│          │                          │
└──────────┴──────────────────────────┘
```

比例：

```
Sidebar
220px fixed

Content
flex
```

不要现在这种：

* 左侧大面积空
* 内容卡片纵向堆叠

---

# 4. Sidebar 设计

现在：

```
概览
配置
节点
链接
请求
规则
DNS
设置
```

太像后台。

改：

```
SATELITE

◉ Dashboard

NETWORK

◎ Nodes
⌁ Connections
⇄ Rules
◇ DNS

SYSTEM

▣ Profiles
▤ Logs
⚙ Settings
```

特点：

* 英文技术词
* 图标辅助
* 分组
* 减少文字感

---

## Sidebar 底部

显示核心状态：

```
────────────────

sing-box

v1.13.5

● Running

```

类似 IDE 状态栏。

---

# 5. 首页 Dashboard

目标：

打开 3 秒知道：

* 有没有运行
* 当前节点
* 网络状态
* 有没有异常

## 第一块：Runtime Header

替代现在巨大状态卡。

```
┌──────────────────────────────┐
│                              │
│ ● RUNNING                    │
│                              │
│ sing-box 1.13.5              │
│ mixed :2080   api :19090     │
│                              │
│                 STOP          │
└──────────────────────────────┘
```

高度：

120px

不要超过。

---

# 6. Runtime Grid

替代现在 8 个 Card。

设计：

```
┌────────┬────────┬────────┬────────┐
│Nodes   │Sub     │Memory  │Uptime  │
│55      │4       │42 MB   │03:22   │
├────────┼────────┼────────┼────────┤
│Upload  │Download│TCP     │UDP     │
│2.3MB/s │12MB/s  │18      │5       │
└────────┴────────┴────────┴────────┘
```

特点：

* 8 个指标
* 一个区域
* 高密度

---

# 13. 动效

少量。

## 启动代理

不要 spinner。

应该：

```
STOPPED

↓

STARTING

↓

RUNNING
```

状态灯变化。

---

## 节点切换

类似终端：

```
Switching Node...

Singapore-04

Latency test

42ms ✓
```

---

# 14. 字体

推荐：

UI：

```
Inter
```

数据：

```
JetBrains Mono
SF Mono
```

例如：

```
sing-box v1.13.5
mixed :2080
42ms
```

全部 mono。

---

# 15. Icon 风格

不要：

* ❌ 彩色 emoji
* ❌ 大插画

使用：

* Lucide
* SF Symbols

线性。

---

# 16. 页面结构最终版

```
Dashboard

 ├ Runtime
 ├ Stats Grid
 ├ Current Node
 └ Connections


Nodes

Connections

 ├ Active
 ├ History


Rules

DNS

Logs


Settings

---

# 17. 最终视觉关键词

如果要一句话描述：

> 「一个给工程师使用的 sing-box 控制台，像 Ghostty 管理网络一样。」

避免：

* 卡片堆叠
* 大圆角
* 蓝色 SaaS 按钮
* 巨大数字
* 空白区域

强化：

* monospace
* 状态
* 密度
* 实时性
* 可诊断性
