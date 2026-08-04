# PRD：DNS 管理模块

## 1. 模块概述

## 1.1 背景

sing-box 具备完整 DNS 路由能力，可以实现：

* 自定义 DNS Server
* DoH / DoT
* FakeIP
* DNS 分流
* DNS 劫持
* 内网域名走系统 DNS
* 防 DNS 泄漏
* TUN 模式下 DNS 接管

本模块目标：

> 提供一个简单但强大的 DNS 管理页面，让用户无需理解 sing-box JSON，即可控制 DNS 行为。

---

# 2. 产品目标

## 用户可以：

* 开启/关闭自定义 DNS
* 配置 DNS Server
* 配置国内/国外 DNS
* 开启 FakeIP
* 配置 DNS 白名单
* 指定域名使用系统 DNS
* 查看 DNS 查询状态
* 避免企业内网 DNS 解析失败

---

# 3. DNS 工作模型

整体流程：

```
              DNS Request

                  |
                  |

              sing-box DNS

                  |

        +---------+----------+

        |                    |

   DNS Rules            Default DNS


        |

 +------+------+


System DNS

Remote DNS

Domestic DNS

FakeIP DNS

```

---

# 4. 页面结构

DNS 设置页面：

```
DNS Settings


[ General ]

[ Servers ]

[ Rules ]

[ FakeIP ]

[ Diagnostics ]

```

---

# 5. General 设置

## 5.1 DNS 模式

选项：

```
DNS Mode


( ) System Only

(●) Smart Mode

( ) Custom Mode

```

### System Only

完全使用系统 DNS：

适合：

* 企业 VPN
* 内网环境

生成：

```json
{
"dns":{
"servers":[
{
"tag":"local",
"address":"local"
}
]
}
}
```

---

### Smart Mode（默认）

自动分流：

```
国内域名

    ↓

国内 DNS


国外域名

    ↓

Remote DNS


内网域名

    ↓

System DNS

```

---

### Custom Mode

用户完全控制。

---

# 6. DNS Server 管理

## 页面

```
DNS Servers


Domestic


✓ AliDNS

223.5.5.5


✓ Tencent

119.29.29.29



Remote


✓ Cloudflare

https://1.1.1.1/dns-query


✓ Google

https://dns.google/dns-query


[Add Custom]

```

---

# 6.1 Server 类型

支持：

## UDP DNS

```
223.5.5.5
```

## TCP DNS

```
tcp://8.8.8.8
```

## DoH

```
https://dns.google/dns-query
```

## DoT

```
tls://1.1.1.1
```

## 系统 DNS

特殊类型：

```
local
```

---

# 7. FakeIP 设置

## 7.1 功能说明

FakeIP：

DNS 返回虚拟地址：

```
example.com

↓

198.18.0.10

↓

sing-box 内部映射

↓

真实 IP

```

优势：

* 避免 DNS 泄漏
* 支持域名路由
* TUN 模式体验更好
* 提升连接速度

---

## 页面

```
FakeIP


Enable FakeIP

[✓]


Address Pool


198.18.0.0/15


IPv4 Only

[✓]


IPv6 FakeIP

[ ]

```

---

# 7.2 FakeIP 白名单

某些域名不应该 FakeIP：

例如：

* 企业内部域名
* 本地服务
* VPN 域名

配置：

```
FakeIP Bypass


*.company.com

*.local

*.lan


[Add]

```

生成：

```json
{
"dns":{
"rules":[
{
"domain_suffix":[
"company.com"
],
"disable_cache":true,
"server":"local"
}
]
}
}
```

---

# 8. DNS 白名单（核心功能）

## 8.1 目的

解决：

```
代理开启

↓

公司内网域名无法访问

↓

因为公网 DNS 不知道内部域名

```

---

## 页面

```
DNS Bypass Rules


Domain


*.corp.company.com


Resolver


System DNS


--------------------------------


git.internal


System DNS



--------------------------------


*.local


System DNS

```

---

# 8.2 支持匹配类型

## Domain

精确：

```
git.company.com
```

---

## Domain Suffix

后缀：

```
company.com
```

匹配：

```
git.company.com

jira.company.com

```

---

## Domain Keyword

包含：

```
internal
```

匹配：

```
api.internal.test
```

---

# 9. DNS Rule 优先级

规则顺序：

```
1. 用户白名单

        ↓

2. FakeIP bypass

        ↓

3. 内网规则

        ↓

4. Geo规则

        ↓

5. 默认 DNS

```

例如：

配置：

```
*.company.com

System DNS


geosite:cn

China DNS


default

Proxy DNS

```

查询：

```
git.company.com

↓

System DNS


baidu.com

↓

China DNS


google.com

↓

Proxy DNS

```

---

# 10. DNS 防泄漏设置

页面：

```
Security


DNS Leak Protection


[✓]


Disable fallback

[✓]


Prevent direct DNS

[✓]

```

说明：

开启后：

```
应用

 |

sing-box

 |

DNS Router

 |

指定 DNS

```

禁止：

```
应用

 |

系统 DNS

```

绕过。

---

# 11. DNS 劫持

TUN 模式：

```
DNS Hijack


[✓] Enable


Listen:

0.0.0.0:53

```

功能：

拦截：

```
8.8.8.8

1.1.1.1

系统 DNS

```

统一进入 sing-box。

---

# 12. DNS Cache

配置：

```
DNS Cache


Enable

[✓]


TTL

300s


Clear Cache

```

用途：

* 减少 DNS 请求
* 提升访问速度

---

# 13. Diagnostics 页面

用于排查：

```
DNS Test


Domain:

git.company.com


Result:


Server:

System DNS


IP:

10.20.1.15


Time:

12ms


```

---

# 14. 内部数据模型

Rust：

```rust
struct DnsSettings {

enabled: bool,

mode: DnsMode,

servers: Vec<DnsServer>,

rules: Vec<DnsRule>,

fake_ip: FakeIpConfig,

hijack: bool,

cache: bool,

}
```

---

## DNS Server

```rust
struct DnsServer {

id:String,

name:String,

address:String,

kind:DnsServerType,

}
```

---

## DNS Rule

```rust
struct DnsRule {


matcher: DomainMatcher,


action: DnsAction,


}


enum DnsAction {


System,


Server(String),


Block,


FakeIp,


}

```

---

# 15. sing-box 配置生成

最终生成：

```json
{
"dns": {

"servers":[

{
"tag":"system",
"address":"local"
},

{
"tag":"remote",
"address":
"https://1.1.1.1/dns-query"
}

],

"rules":[

{
"domain_suffix":[
"corp.company.com"
],
"server":"system"
},

{
"geosite":"cn",
"server":"cn"
}

],

"final":"remote"

}

}
```

---

# 16. 默认配置建议

新安装：

```
DNS Mode:

Smart


Servers:

System DNS

+

223.5.5.5

+

1.1.1.1 DoH


FakeIP:

Enabled


DNS WhiteList:

.local

.lan

.internal

.corp


DNS Leak Protection:

Enabled

```

---

# 17. 与规则系统关系

最终架构：

```
                 Profile


                    |

        +-----------+------------+

        |                        |

    DNS Engine              Route Engine


        |                        |

 DNS Resolver             Proxy Decision


        |                        |

 sing-box DNS             sing-box Route

```

---

# 18. MVP 实现范围

## 第一版必须：

* DNS Server 配置
* System DNS
* DoH
* DNS Rules
* 白名单
* FakeIP 开关
* FakeIP bypass
* DNS Cache
* DNS 测试

## 第二版：

* DNS 查询日志
* GeoDNS
* 自动检测内网 DNS
* 每个网络环境独立配置
* VPN 感知

---

这个 DNS 模块会成为你这个客户端区别于普通 Clash GUI 的核心能力之一。尤其是“DNS 白名单 + System DNS + FakeIP bypass”组合，可以很好覆盖企业网络、家庭网络、代理网络三类场景。

