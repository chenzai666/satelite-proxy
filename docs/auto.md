## 结论

业界更常见的方案是：

> **真实流量被动观测作为触发器，主动探测负责确认和选择候选节点，再配合滞回、冷却和熔断控制切换频率。**

不建议持续轮询全部节点，也不建议仅凭某个请求变慢立即切换。Envoy、HAProxy 等成熟代理体系都会组合被动健康检查与主动健康检查；Mihomo 也支持 lazy 检测，并在失败累计后强制执行健康检查。([HAProxy Technologies][1])

---

## 一、整体架构

建议拆成四个模块：

```text
sing-box 流量
    │
    ▼
Passive Observer
真实连接指标聚合
    │
    ▼
Degradation Detector
判断当前节点是否退化
    │
    ▼
Active Prober
按需探测少量候选节点
    │
    ▼
Switch Controller
评分、切换、冷却、回切
```

核心原则：

1. 日常阶段主要收集真实流量指标。
2. 当前节点疑似异常时，才启动主动探测。
3. 首轮只探测历史表现最好的 3～5 个节点。
4. 候选节点明确优于当前节点时切换。
5. 切换后设置最短驻留时间，防止来回跳动。

---

# 二、真实流量应该监控什么

## 1. 强信号

这些信号很适合判断代理节点异常：

* TCP 建连失败
* 代理握手失败
* TLS 握手失败
* 首字节前连接重置
* 首字节超时
* UDP 连续无响应
* QUIC 握手失败
* 连接刚建立便异常关闭
* 当前节点连续多个不同域名失败

其中应重点区分：

```text
本地产生的错误
├─ dial timeout
├─ connection refused
├─ connection reset
├─ network unreachable
└─ TLS handshake timeout

目标网站产生的错误
├─ HTTP 404
├─ HTTP 500
├─ 网站限流
└─ 业务接口自身变慢
```

节点切换主要依据本地产生的连接错误。Envoy 的 outlier detection 也支持单独统计 local-origin failure，例如连接失败、重置、超时等，避免目标服务自身的错误污染节点健康度。([Envoy Proxy][2])

## 2. 性能信号

建议每条连接记录：

```rust
struct ConnectionSample {
    node_id: String,
    destination: String,

    dial_ms: Option<u32>,
    handshake_ms: Option<u32>,
    first_byte_ms: Option<u32>,

    duration_ms: u64,
    uploaded_bytes: u64,
    downloaded_bytes: u64,

    error: Option<ConnectionError>,
    network_type: NetworkType,
}
```

比较重要的三个时间：

### `dial_ms`

连接代理服务器和建立代理隧道所需时间。

这个指标最能体现节点入口质量。

### `first_byte_ms`

请求发出后，首次收到远端数据的时间。

它包含：

* 节点入口延迟
* 节点出口质量
* 目标网站响应时间

因此需要跨多个不同域名聚合，不能单独拿某个网站判断。

### `duration_ms`

不能直接用于节点评分。

视频、下载、WebSocket、SSH 等连接天然持续很久。连接持续时间高，不代表节点质量差。

---

# 三、HTTPS 下的监控限制

在 TUN 或普通代理模式下，你通常能够获得：

* 目标域名或 SNI
* 目标 IP
* TCP 建连时间
* 首次上行时间
* 首次下行时间
* 上传下载字节数
* 连接持续时间
* 超时、重置、关闭原因

通常无法可靠获得：

* HTTPS HTTP 状态码
* 精确的 HTTP 请求数量
* HTTP 层 TTFB
* 同一个 HTTP/2 连接中每个请求的响应时间
* QUIC 内部每条 HTTP/3 请求的状态

除非实现 TLS MITM，但代理软件一般不应为了自动选节点引入 MITM。

因此建议使用“连接级指标”，不要宣传成“所有 HTTP 请求级监控”。

HTTP/2、HTTP/3 和连接复用还会带来一个问题：一个连接内部可能承载大量请求，连接级首字节只能反映初始阶段。可通过以下指标补充：

* 活跃连接期间的连续无下行时间
* 滑动窗口吞吐量
* TCP reset
* QUIC connection close
* 应用重连次数
* 同一域名短时间反复新建连接

---

# 四、退化检测算法

建议为当前节点维护多个滑动窗口：

```text
快速窗口：最近 10 秒
短期窗口：最近 60 秒
基线窗口：最近 15 分钟
```

每个窗口维护：

```text
connect_success_rate
consecutive_failures
dial_ewma
first_byte_ewma
p90_first_byte
reset_rate
timeout_rate
sample_count
destination_count
```

## 推荐触发条件

### 立即触发

任意条件满足：

```text
连续 3 次连接失败
10 秒内 5 次本地超时
当前节点服务器 TCP 连接失败
代理握手连续失败
网络接口发生切换
系统网络重新连接
```

### 软触发

需要满足最小样本量，例如最近 30 秒至少 8 条有效连接：

```text
失败率 >= 25%
```

或者：

```text
当前 first_byte_ewma > 基线的 2.5 倍
并且绝对增量 > 300ms
```

或者：

```text
p90_first_byte > 1500ms
且涉及至少 3 个不同域名
```

要求“多个域名”非常重要。某个网站自身故障时，不应触发全局节点切换。

---

# 五、EWMA 比普通平均值更合适

可以使用指数移动平均：

```rust
fn update_ewma(previous: f64, sample: f64, alpha: f64) -> f64 {
    alpha * sample + (1.0 - alpha) * previous
}
```

建议：

```text
dial EWMA alpha = 0.25
first-byte EWMA alpha = 0.15
failure EWMA alpha = 0.30
```

较新的异常会快速影响结果，偶发尖峰又不会直接引发切换。

失败事件可以映射成数值：

```text
成功            0.0
慢响应          0.4
首字节超时      0.8
连接失败        1.0
握手失败        1.0
```

---

# 六、主动探测不要一次扫描全部节点

推荐分级探测。

## Level 0：正常状态

* 只使用真实流量观测。
* 当前节点长时间无流量时暂停检测。
* 候选节点使用最近一次结果缓存。

sing-box 的 URLTest 自带 `idle_timeout`，默认空闲 30 分钟后暂停周期测试；默认测试周期为 3 分钟，默认切换容差为 50ms。([Sing Box][3])

## Level 1：确认当前节点

发现退化后，先对当前节点进行两个轻量探测：

```text
探测地址 A：204 HTTP endpoint
探测地址 B：另一运营商或 CDN endpoint
并发数：2
超时：2～3 秒
```

使用两个地址可以降低单一目标服务故障造成的误判。

探测内容可以包含：

1. TCP 建连
2. TLS 握手
3. HTTP 204
4. 下载 16KB～64KB 小文件

单纯 TCP ping 只能验证入口；HTTP 小文件能覆盖代理入口、代理出口和基本吞吐。

## Level 2：探测 Top K 候选

当前节点确认异常后，只探测：

```text
历史评分最好的 3～5 个节点
同地区节点优先
最近成功使用过的节点优先
已熔断节点排除
```

并发建议控制在 3 或 4。

## Level 3：扩大搜索

首批候选全部不可用时，再批量探测其他节点：

```text
每批 5 个
批间间隔 200～500ms
找到合格节点后立即停止
```

这种方式能避免订阅包含几十或几百个节点时产生探测风暴。

Mihomo 的健康检查支持 `lazy`，未选中的策略组可以暂停检查；达到最大失败次数后触发强制健康检查。这与事件驱动探测思路接近。([Metacubex Wiki][4])

---

# 七、节点评分

先进行可用性过滤：

```text
探测失败             淘汰
TLS 握手失败         淘汰
失败率过高           淘汰
正在熔断             淘汰
```

再计算质量评分：

```text
score =
    0.30 × latency_ratio
  + 0.25 × first_byte_ratio
  + 0.25 × failure_penalty
  + 0.10 × jitter_ratio
  + 0.10 × throughput_penalty
```

示例：

```rust
struct NodeScore {
    latency_ratio: f64,
    first_byte_ratio: f64,
    failure_penalty: f64,
    jitter_ratio: f64,
    throughput_penalty: f64,
}

impl NodeScore {
    fn total(&self) -> f64 {
        self.latency_ratio * 0.30
            + self.first_byte_ratio * 0.25
            + self.failure_penalty * 0.25
            + self.jitter_ratio * 0.10
            + self.throughput_penalty * 0.10
    }
}
```

各项建议使用相对值：

```text
latency_ratio = candidate_latency / reference_latency
```

比固定要求“延迟必须低于 100ms”更加适合不同地区和不同网络。

---

# 八、切换条件要带滞回

假设当前节点延迟 130ms，候选节点 110ms，立即切换没有太大意义，还可能导致频繁抖动。

建议切换条件：

```text
硬故障：
候选节点只要可用，立即切换

软退化：
候选节点评分至少改善 25%
或延迟改善至少 80～100ms
并连续两次探测通过
```

可以表示为：

```rust
let significantly_better =
    candidate_score <= current_score * 0.75
    || current_latency_ms.saturating_sub(candidate_latency_ms) >= 100;
```

sing-box URLTest 也有 `tolerance`，用于防止两个延迟接近的节点频繁互换。默认值是 50ms。([Sing Box][5])

---

# 九、冷却、熔断和回切

## 最短驻留时间

切换完成后：

```text
普通切换：至少保持 2 分钟
网络波动：至少保持 5 分钟
硬故障切换：允许再次快速切换
```

## 节点熔断

节点连续失败后暂时排除：

```text
第一次熔断：30 秒
第二次熔断：2 分钟
第三次熔断：10 分钟
后续：最多 30 分钟
```

熔断结束后先进入 `half-open`：

```text
只允许主动探测
连续成功 2 次后恢复候选资格
```

Envoy 的 outlier detection 同样会根据连续失败、失败比例和成功率异常将节点临时移出健康集合。([Envoy Proxy][2])

## 回切

不要因为原先优先节点刚恢复就马上回切。

建议：

```text
原节点连续健康 3 次
评分至少比当前节点好 20%
当前节点已使用超过 5 分钟
```

---

# 十、需要识别“节点问题”和“本地网络问题”

这是自动切换中最容易忽略的一点。

当当前节点变慢后，探测 3 个候选节点：

```text
当前节点差，候选节点正常
=> 节点问题，可以切换

当前节点差，所有候选节点都差
=> Wi-Fi、运营商、系统网络或目标 CDN 问题
=> 暂缓切换

只有某个域名差
=> 目标网站问题
=> 不切换
```

可以计算相对退化：

```text
current_degradation = 当前节点当前延迟 / 当前节点历史基线
global_degradation  = 候选节点中位延迟 / 候选节点历史基线
```

若两者同时明显升高，说明整体网络环境正在恶化。

界面上也可以显示：

```text
Current node degraded
Local network degraded
Destination service degraded
```

---

# 十一、在 sing-box 中怎样接入

## 方案 A：Selector + 外部控制器

建议把自动切换策略放在你的 Rust/Tauri 进程中，sing-box 只负责数据面。

配置一个 selector：

```json
{
  "type": "selector",
  "tag": "proxy-auto",
  "outbounds": [
    "node-a",
    "node-b",
    "node-c"
  ],
  "default": "node-a",
  "interrupt_exist_connections": false
}
```

由你的控制器通过 API 修改当前选中节点。

sing-box 官方文档说明，Selector 可以通过 Clash API 控制；切换时也支持选择是否中断现有入站连接。([Sing Box][6])

推荐默认：

```json
"interrupt_exist_connections": false
```

这样：

* 新连接使用新节点。
* 下载、视频、SSH、WebSocket 尽量继续使用旧连接。
* 避免切换导致所有应用瞬间断开。

硬故障时，旧连接本身通常已经无法使用。你可以单独关闭属于故障节点的连接，不必全量中断。

## 方案 B：直接使用 URLTest

适合基础自动选择：

```json
{
  "type": "urltest",
  "tag": "proxy-auto",
  "outbounds": [
    "node-a",
    "node-b",
    "node-c"
  ],
  "url": "https://www.gstatic.com/generate_204",
  "interval": "10m",
  "idle_timeout": "30m",
  "tolerance": 100,
  "interrupt_exist_connections": false
}
```

它的不足：

* 依据固定测试 URL。
* 无法直接纳入真实连接失败率。
* 无法区分网站变慢、节点变慢和本地网络变慢。
* 很难实现自定义熔断、候选分批、动态阈值。
* 评分维度主要是测试延迟。

因此你的软件要实现“真实流量触发的智能切换”，更适合 Selector + 自己的策略控制器。URLTest 可以作为探测工具或基础模式。

## API 与连接数据

sing-box 的 Clash API 提供 RESTful 控制入口；V2Ray API 可以统计指定入站和出站的流量。([Sing Box][7])

sing-box 1.14 分支新增了 gRPC API，能力包含：

* 服务状态
* 日志
* 出站组选择
* URL 测试
* 连接跟踪
* 网络质量测试

但 1.14 当前仍需要结合你实际使用的发布渠道和构建版本评估，生产实现可以先兼容 Clash API。([Sing Box][8])

---

# 十二、是否需要修改 sing-box 内核

如果仅需要：

* 节点延迟
* 连接列表
* 上传下载流量
* 目标域名
* 当前出站
* 主动 URL 测试
* 切换 selector

可以先使用现有 API，不修改内核。

如果需要准确获取：

* 每条连接的 dial duration
* 代理握手 duration
* 首次下行字节时间
* 具体关闭原因
* TCP reset 与 timeout 分类
* UDP 会话丢包或无响应
* 连接实际使用的最终节点
* multiplex 内部 stream 级数据

建议给 sing-box 增加一个轻量事件接口，或者以 library 方式嵌入并在关键位置挂 observer。

事件可以设计成：

```rust
enum ProxyEvent {
    ConnectionOpened {
        id: u64,
        node: String,
        destination: String,
        timestamp: Instant,
    },
    OutboundConnected {
        id: u64,
        dial_duration: Duration,
    },
    FirstUpstreamByte {
        id: u64,
        elapsed: Duration,
    },
    ConnectionClosed {
        id: u64,
        uploaded: u64,
        downloaded: u64,
        error: Option<ProxyError>,
    },
}
```

这些事件通过 bounded channel 发送给聚合线程：

```text
连接热路径
    │ try_send
    ▼
有界事件队列
    │
    ▼
单独聚合线程
    │
    ▼
每秒输出 NodeSnapshot
```

不要在连接热路径执行数据库写入、JSON 序列化或复杂评分。

---

# 十三、推荐的状态机

```text
HEALTHY
  │ 真实流量异常
  ▼
SUSPECTED
  │ 主动确认失败
  ▼
PROBING
  │ 找到明显更好的候选
  ▼
SWITCHING
  │ 切换成功
  ▼
COOLDOWN
  │ 冷却结束
  ▼
HEALTHY
```

异常恢复路径：

```text
SUSPECTED
  │ 当前节点主动探测正常
  ▼
HEALTHY
```

全部节点异常：

```text
PROBING
  │ 所有候选同时变差
  ▼
NETWORK_DEGRADED
```

---

# 十四、一套可以直接使用的初始参数

```yaml
passive_monitor:
  fast_window: 10s
  short_window: 60s
  baseline_window: 15m
  minimum_samples: 8
  minimum_destinations: 3

degradation:
  consecutive_failures: 3
  failure_rate: 0.25
  first_byte_ratio: 2.5
  first_byte_absolute_increase: 300ms
  p90_first_byte: 1500ms

active_probe:
  initial_candidates: 4
  concurrency: 3
  timeout: 2500ms
  attempts: 2
  endpoints:
    - https://www.gstatic.com/generate_204
    - configurable-secondary-endpoint

switch:
  minimum_improvement_ratio: 0.25
  minimum_latency_improvement: 100ms
  minimum_dwell: 2m
  cooldown: 90s

circuit_breaker:
  first_ejection: 30s
  second_ejection: 2m
  third_ejection: 10m
  maximum_ejection: 30m
  recovery_successes: 2
```

## 最推荐的最终方案

你的软件可以实现两个自动模式：

### 基础自动模式

直接使用 sing-box URLTest：

```text
低实现成本
适合普通用户
固定 URL 延迟选择
```

### 智能自动模式

Rust 侧实现：

```text
真实连接被动观测
→ 退化触发
→ 当前节点确认
→ Top 4 候选按需探测
→ 相对评分
→ 带滞回切换
→ 冷却和熔断
```

第二种方案更接近成熟负载均衡器和高可用代理的健康管理逻辑，也更符合你“不频繁轮询全部节点”的要求。

[1]: https://www.haproxy.com/documentation/haproxy-configuration-tutorials/reliability/health-checks/?utm_source=chatgpt.com "Health checks | HAProxy config tutorials"
[2]: https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/outlier.html?utm_source=chatgpt.com "Outlier detection — envoy 1.40.0-dev-4ec588 documentation"
[3]: https://sing-box.sagernet.org/zh/configuration/outbound/urltest/?utm_source=chatgpt.com "URLTest - sing-box"
[4]: https://wiki.metacubex.one/en/config/proxy-groups/?utm_source=chatgpt.com "proxy-groups configuration - mihomo docs"
[5]: https://sing-box.sagernet.org/configuration/outbound/urltest/?utm_source=chatgpt.com "URLTest - sing-box"
[6]: https://sing-box.sagernet.org/configuration/outbound/selector/?utm_source=chatgpt.com "Selector - sing-box"
[7]: https://sing-box.sagernet.org/zh/configuration/experimental/clash-api/?utm_source=chatgpt.com "Clash API - sing-box"
[8]: https://sing-box.sagernet.org/zh/configuration/service/api/?utm_source=chatgpt.com "sing-box API - sing-box"

