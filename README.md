<div align="center">

# CloudflareST-Rust

**对 [XIU2/CloudflareSpeedTest](https://github.com/XIU2/CloudflareSpeedTest) 使用 Rust 重写**

![Rust Version](https://img.shields.io/badge/rustc-latest-orange?style=flat-square&logo=rust)
[![zread](https://img.shields.io/badge/Ask_Zread-_.svg?style=flat&color=00b0aa&labelColor=000000&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB3aWR0aD0iMTYiIGhlaWdodD0iMTYiIHZpZXdCb3g9IjAgMCAxNiAxNiIgZmlsbD0ibm9uZSIgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIj4KPHBhdGggZD0iTTQuOTYxNTYgMS42MDAxSDIuMjQxNTZDMS44ODgxIDEuNjAwMSAxLjYwMTU2IDEuODg2NjQgMS42MDE1NiAyLjI0MDFWNC45NjAxQzEuNjAxNTYgNS4zMTM1NiAxLjg4ODEgNS42MDAxIDIuMjQxNTYgNS42MDAxSDQuOTYxNTZDNS4zMTUwMiA1LjYwMDEgNS42MDE1NiA1LjMxMzU2IDUuNjAxNTYgNC45NjAxVjIuMjQwMUM1LjYwMTU2IDEuODg2NjQgNS4zMTUwMiAxLjYwMDEgNC45NjE1NiAxLjYwMDFaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00Ljk2MTU2IDEwLjM5OTlIMi4yNDE1NkMxLjg4ODEgMTAuMzk5OSAxLjYwMTU2IDEwLjY4NjQgMS42MDE1NiAxMS4wMzk5VjEzLjc1OTlDMS42MDE1NiAxNC4xMTM0IDEuODg4MSAxNC4zOTk5IDIuMjQxNTYgMTQuMzk5OUg0Ljk2MTU2QzUuMzE1MDIgMTQuMzk5OSA1LjYwMTU2IDE0LjExMzQgNS42MDE1NiAxMy43NTk5VjExLjAzOTlDNS42MDE1NiAxMC42ODY0IDUuMzE1MDIgMTAuMzk5OSA0Ljk2MTU2IDEwLjM5OTlaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik0xMy43NTg0IDEuNjAwMUgxMS4wMzg0QzEwLjY4NSAxLjYwMDEgMTAuMzk4NCAxLjg4NjY0IDEwLjM5ODQgMi4yNDAxVjQuOTYwMUMxMC4zOTg0IDUuMzEzNTYgMTAuNjg1IDUuNjAwMSAxMS4wMzg0IDUuNjAwMUgxMy43NTg0QzE0LjExMTkgNS42MDAxIDE0LjM5ODQgNS4zMTM1NiAxNC4zOTg0IDQuOTYwMVYyLjI0MDFDMTQuMzk4NCAxLjg4NjY0IDE0LjExMTkgMS42MDAxIDEzLjc1ODQgMS42MDAxWiIgZmlsbD0iI2ZmZiIvPgo8cGF0aCBkPSJNNCAxMkwxMiA0TDQgMTJaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00IDEyTDEyIDQiIHN0cm9rZT0iI2ZmZiIgc3Ryb2tlLXdpZHRoPSIxLjUiIHN0cm9rZS1saW5lY2FwPSJyb3VuZCIvPgo8L3N2Zz4K&logoColor=ffffff)](https://zread.ai/GuangYu-yu/CloudflareST-Rust)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust)
[![ReadmeX](https://raw.githubusercontent.com/CodePhiliaX/resource-trusteeship/main/readmex.svg)](https://readmex.com/GuangYu-yu/CloudflareST-Rust)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL%20v3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![GitHub Star](https://img.shields.io/github/stars/GuangYu-yu/CloudflareST-Rust.svg?style=flat-square&label=Star&color=00ADD8&logo=github)](https://github.com/GuangYu-yu/CloudflareST-Rust/)
[![GitHub Fork](https://img.shields.io/github/forks/GuangYu-yu/CloudflareST-Rust.svg?style=flat-square&label=Fork&color=00ADD8&logo=github)](https://github.com/GuangYu-yu/CloudflareST-Rust/)

**⚠️ 警告：工具仅用于简单的网络测速，造成的一切后果自负**

</div>

## 📝 使用建议

- 建议从大范围 CIDR 中指定较大测速数量，并使用 `-tn` 参数
  - 例如：`-ip 2606:4700::/48=1000 -tn 300`
  - 含义是：对 2606:4700::/48 最多测速 1000 个随机 IP，并在延迟测速到 300 个可用 IP 后直接进行下一步
- 因为采取了流式处理，每个 IP 都实时生成、测速并过滤，内存中始终只有符合要求的结果

## ✨ 功能特点

- 📊 下载测速期间，显示实时速度
- ⚡ IP 的生成和测速都是流式处理的，对 CIDR 依据采样数量均匀分割
- 🔌 优先使用指定端口测速，例如：`-ip [2606:4700::]:8080,104.16.0.0:80`
- 🔗 支持从指定 URL 中获取测速 CIDR/IP 列表（`-ipurl`）
- 📋 支持从指定 URL 中获取测速地址列表（`-urlist`）
- 🌐 支持绑定到指定 IP 或接口名进行测速
- ⏱️ 支持给程序限制运行时间，超时后立即结算结果并退出
- 🔄 使用 -httping 参数时，通过 `http://<IP>/cdn-cgi/trace` 进行测速

## 🚀 示例命令

```bash
-ipurl https://www.cloudflare.com/ips-v4 -tn 3000 -dn 10 -sl 15 -tlr 0 -hu cp.cloudflare.com -url https://speed.cloudflare.com/__down?bytes=524288000
```

> [!IMPORTANT]
>- `speed.cloudflare.com` 不允许非 TLS 的 HTTP 下载测速，需[自建](https://github.com/GuangYu-yu/CF-Workers-SpeedTestURL)测速地址
>- -hu 参数指定 HTTPS 延迟测速的 URL 地址，如果不带值则与下载测速共用地址
>- 下载持续时间太短则不会算作有效速度，需确保下载测速文件足够大
>- 多网卡情况下，使用 -intf 参数会被策略路由影响效果
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
| `-intf` | 绑定到指定的网络接口或 IP 进行测速 | eth0 or pppoe-ct | 未指定 |

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

<img width="780" height="380" alt="演示图" src="https://github.com/user-attachments/assets/e202c2bb-668b-44d7-82f5-79bf80c9a3d2" />

> 这里 `x|y` 的含义是已进行下载测速 y 个，获取到 x 个符合要求的结果

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
bash -c 'ARCH=$( [ "$(uname -m)" = x86_64 ] && echo amd64 || echo arm64 ); curl -fsSL https://github.com/GuangYu-yu/CloudFlare-DDNS/releases/download/setup/setup.sh | bash -s -- GuangYu-yu CloudflareST-Rust main-latest CloudflareST-Rust_linux_$ARCH.tar.gz CloudflareST-Rust'
```

> - 安卓下载 [Termux](https://github.com/termux/termux-app/releases)

或者可使用 [工具](https://github.com/GuangYu-yu/CloudFlare-DDNS)，能测速并解析到 Cloudflare 或提交到 Github：

```bash
curl -ksSL https://github.com/GuangYu-yu/CloudFlare-DDNS/releases/download/setup/cfopw.sh | bash
```

```bash
bash -c 'ARCH=$( [ "$(uname -m)" = x86_64 ] && echo amd64 || echo arm64 ); curl -fsSL https://github.com/GuangYu-yu/CloudFlare-DDNS/releases/download/setup/setup.sh | bash -s -- GuangYu-yu CloudflareST-Rust main-latest CloudflareST-Rust_linux_$ARCH.tar.gz CloudflareST-Rust GuangYu-yu CloudFlare-DDNS main-latest CFRS_linux_$ARCH.tar.gz CFRS'
```
