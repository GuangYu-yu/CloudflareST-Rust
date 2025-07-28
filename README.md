<div align="center">

# CloudflareST-Rust

**对 [XIU2/CloudflareSpeedTest](https://github.com/XIU2/CloudflareSpeedTest) 使用 Rust 重写**

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL%20v3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![GitHub Star](https://img.shields.io/github/stars/GuangYu-yu/CloudflareST-Rust.svg?style=flat-square&label=Star&color=00ADD8&logo=github)](https://github.com/GuangYu-yu/CloudflareST-Rust/)
[![GitHub Fork](https://img.shields.io/github/forks/GuangYu-yu/CloudflareST-Rust.svg?style=flat-square&label=Fork&color=00ADD8&logo=github)](https://github.com/GuangYu-yu/CloudflareST-Rust/)

**⚠️ 警告：工具仅用于简单的网络测速，造成的一切后果自负**

</div>

## 📝 使用建议

- 建议从大范围 CIDR 中指定较大测速数量，并使用 `-tn` 参数
  - 例如：`-ip 2606:4700::/48=100000 -tn 30000`
  - 含义是：对 2606:4700::/48 最多测速 100000 个随机 IP，并在测速到 30000 个可用 IP 后立即结算
- 因为采取了流式处理，每个 IP 都实时生成、测速并过滤，内存中始终只有符合要求的结果

## ✨ 功能特点

- ⚡ IP 的生成和测速都是流式处理的
- 📊 下载测速期间，显示实时速度
- 🔌 优先使用指定端口测速，例如：`-ip [2606:4700::]:8080,104.16.0.0:80`
- 🔗 支持从指定 URL 中获取测速 CIDR/IP 列表（`-ipurl`）
- 📋 支持从指定 URL 中获取测速地址列表（`-urlist`）
- 🌐 使用了 Httping 或下载测速之后，会在结果显示数据中心
- ⏱️ 支持给程序限制运行时间，超时后立即结算结果并退出
- 🔄 使用 -httping 参数时，通过 `http://<IP>/cdn-cgi/trace` 进行测速

## 🚀 示例命令

```bash
-ip 2606:4700:100::/48=10000,2606:4700:102::/48=10000 -tn 5000 -dn 10 -sl 15 -hu cp.cloudflare.com -url https://speed.cloudflare.com/__down?bytes=524288000
```

> [!IMPORTANT]
>- `speed.cloudflare.com` 无法进行 HTTP 下载测速，需[自建](https://github.com/GuangYu-yu/CF-Workers-SpeedTestURL)测速地址
>- -hu 参数指定 HTTPS 延迟测速的 URL 地址，如果不带值则与下载测速共用地址
>- 下载持续时间太短则不会算作有效速度，需确保下载测速文件足够大
>- 注意队列数量和实时下载速度，设置合理的筛选条件
>- 可用 IP 数量是 Ping 通的，并非经历过筛选的数量
>- 如果不想写入文件，直接使用 -o 参数并不带值即可

## 📋 参数说明

### 基本参数

| 参数 | 说明 | 示例 | 默认值 |
|:-----|:-----|:-------|:-------|
| `-url` | TLS 模式的 Httping 或下载测速所使用的测速地址 | https://example.com/file | 未指定 |
| `-urlist` | 从 URL 内读取测速地址列表 | https://example.com | 未指定 |
| `-f` | 从文件或文件路径读取 IP 或 CIDR | ip.txt | 未指定 |
| `-ip` | 直接指定 IP 或 CIDR（多个用逗号分隔） | 104.16.0.0/13=500,2606:4700::/36 | 未指定 |
| `-ipurl` | 从URL读取 IP 或 CIDR | https://www.cloudflare.com/ips-v4 | 未指定 |
| `-timeout` | 程序超时退出时间（秒） | 3600 | 不限制 |

### 测速参数

| 参数 | 说明 | 默认值 |
|:-----|:-----|:-------|
| `-t` | 延迟测速次数 | 4 |
| `-dn` | 下载测速所需符合要求的结果数量 | 10 |
| `-dt` | 下载测速时间（秒） | 10 |
| `-tp` | 测速端口 | 443 |
| `-all4` | 测速全部 IPv4 | 否 |
| `-tn` | 当 Ping 到指定可用数量，提前结束 Ping | 否 |

### 测速选项

| 参数 | 说明 | 示例 | 默认值 |
|:-----|:-----|:-------|:-------|
| `-httping` | 使用非 TLS 模式的 Httping | N/A | 否 |
| `-dd` | 禁用下载测速 | N/A | 否 |
| `-hc` | 指定 HTTPing 的状态码 | 200,301,302 | 未指定 |
| `-hu` | 使用 HTTPS 进行延迟测速，可指定测速地址 | None or https://cp.cloudflare.com | 否 |
| `-colo` | 匹配指定地区 | HKG,sjc | 未指定 |
| `-n` | 延迟测速的线程数量 | N/A | 256 |

### 结果参数

| 参数 | 说明 | 默认值 |
|:-----|:-----|:-------|
| `-tl` | 延迟上限（毫秒） | 2000 |
| `-tll` | 延迟下限（毫秒） | 0 |
| `-tlr` | 丢包率上限 | 1.00 |
| `-sl` | 下载速度下限（MB/s） | 0.00 |
| `-p` | 终端显示结果数量 | 10 |
| `-sp` | 结果中带端口号 | 否 |
| `-o` | 输出结果文件（文件名或文件路径） | result.csv |

## 📊 测速结果示例

```
开始延迟测速（模式：Tcping, 端口：443, 范围：0 ~ 300 ms, 丢包：0.20)
30000/30000 [==========================================↖] 可用：10811
开始下载测速（下限：15.00 MB/s, 所需：10, 队列：10811）
10/10 [=================================================↘] 15.58 MB/s
IP 地址           已发送  已接收  丢包率    平均延迟    下载速度(MB/s)   数据中心
104.25.---.--     8       8       0.00      65.72      20.18              LAX
104.18.---.---    8       8       0.00      99.94      19.36              SJC
104.25.---.---    8       8       0.00      84.28      19.08              LAX
104.25.---.--     8       8       0.00      98.01      18.91              LAX
172.64.---.--     8       7       0.12     100.44      18.47              LAX
104.25.---.--     8       8       0.00      95.47      18.44              FRA
104.25.---.---    8       8       0.00      97.92      18.35              LAX
104.25.---.---    8       7       0.12     100.66      18.29              LAX
104.25.---.--     8       7       0.12     100.47      18.21              FRA
104.25.---.---    8       8       0.00      99.70      18.18              FRA

[信息] 测速结果已写入 result.csv 文件，可使用记事本/表格软件查看
程序执行完毕


```

## 📥 下载链接

| 平台   | 架构   | 下载链接                                                                 |
|:-------|:-------|:--------------------------------------------------------------------------|
| Linux  | AMD64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/Linux_AMD64/CloudflareST-Rust)   |
| Linux  | ARM64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/Linux_ARM64/CloudflareST-Rust)   |
| MacOS  | AMD64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/MacOS_AMD64/CloudflareST-Rust)   |
| MacOS  | ARM64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/MacOS_ARM64/CloudflareST-Rust)   |
| Windows| AMD64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/Windows_AMD64/CloudflareST-Rust.exe) |
| Windows| ARM64  | [下载](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/Windows_ARM64/CloudflareST-Rust.exe) |

## 📱 安装方法

### 安卓/OpenWrt 安装

如果仅获取 `CloudflareST-Rust`，可使用：

```bash
curl -ksSL https://raw.githubusercontent.com/GuangYu-yu/opw-cloudflare/refs/heads/main/setup_cloudflarest.sh | bash
```

> - 安卓下载 [Termux](https://github.com/termux/termux-app/releases)

或者可使用 [工具](https://github.com/GuangYu-yu/opw-cloudflare)，能测速并解析到 Cloudflare 或提交到 Github：

```bash
curl -ksSL https://raw.githubusercontent.com/GuangYu-yu/opw-cloudflare/main/cfopw.sh | bash
```

`bash cf` 进入菜单