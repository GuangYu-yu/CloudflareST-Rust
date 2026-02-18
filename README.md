<div align="center">

# CloudflareST-Rust

**对 [XIU2/CloudflareSpeedTest](https://github.com/XIU2/CloudflareSpeedTest) 使用 Rust 重写**

![Rust Version](https://img.shields.io/badge/rustc-latest-orange?style=flat-square&logo=rust)
[![zread](https://img.shields.io/badge/Ask_Zread-_.svg?style=flat&color=00b0aa&labelColor=000000&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB3aWR0aD0iMTYiIGhlaWdodD0iMTYiIHZpZXdCb3g9IjAgMCAxNiAxNiIgZmlsbD0ibm9uZSIgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIj4KPHBhdGggZD0iTTQuOTYxNTYgMS42MDAxSDIuMjQxNTZDMS44ODgxIDEuNjAwMSAxLjYwMTU2IDEuODg2NjQgMS42MDE1NiAyLjI0MDFWNC45NjAxQzEuNjAxNTYgNS4zMTM1NiAxLjg4ODEgNS42MDAxIDIuMjQxNTYgNS42MDAxSDQuOTYxNTZDNS4zMTUwMiA1LjYwMDEgNS42MDE1NiA1LjMxMzU2IDUuNjAxNTYgNC45NjAxVjIuMjQwMUM1LjYwMTU2IDEuODg2NjQgNS4zMTUwMiAxLjYwMDEgNC45NjE1NiAxLjYwMDFaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00Ljk2MTU2IDEwLjM5OTlIMi4yNDE1NkMxLjg4ODEgMTAuMzk5OSAxLjYwMTU2IDEwLjY4NjQgMS42MDE1NiAxMS4wMzk5VjEzLjc1OTlDMS42MDE1NiAxNC4xMTM0IDEuODg4MSAxNC4zOTk5IDIuMjQxNTYgMTQuMzk5OUg0Ljk2MTU2QzUuMzE1MDIgMTQuMzk5OSA1LjYwMTU2IDE0LjExMzQgNS42MDE1NiAxMy43NTk5VjExLjAzOTlDNS42MDE1NiAxMC42ODY0IDUuMzE1MDIgMTAuMzk5OSA0Ljk2MTU2IDEwLjM5OTlaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik0xMy43NTg0IDEuNjAwMUgxMS4wMzg0QzEwLjY4NSAxLjYwMDEgMTAuMzk4NCAxLjg4NjY0IDEwLjM5ODQgMi4yNDAxVjQuOTYwMUMxMC4zOTg0IDUuMzEzNTYgMTAuNjg1IDUuNjAwMSAxMS4wMzg0IDUuNjAwMUgxMy43NTg0QzE0LjExMTkgNS42MDAxIDE0LjM5ODQgNS4zMTM1NiAxNC4zOTg0IDQuOTYwMVYyLjI0MDFDMTQuMzk4NCAxLjg4NjY0IDE0LjExMTkgMS42MDAxIDEzLjc1ODQgMS42MDAxWiIgZmlsbD0iI2ZmZiIvPgo8cGF0aCBkPSJNNCAxMkwxMiA0TDQgMTJaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00IDEyTDEyIDQiIHN0cm9rZT0iI2ZmZiIgc3Ryb2tlLXdpZHRoPSIxLjUiIHN0cm9rZS1saW5lY2FwPSJyb3VuZCIvPgo8L3N2Zz4K&logoColor=ffffff)](https://zread.ai/GuangYu-yu/CloudflareST-Rust)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust)
[![ReadmeX](https://raw.githubusercontent.com/CodePhiliaX/resource-trusteeship/main/readmex.svg)](https://readmex.com/GuangYu-yu/CloudflareST-Rust)
<p align="center">
  <img src="https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/main/badges/star.svg" height="24">
  <img src="https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/main/badges/fork.svg" height="24">
</p>

**⚠️ 警告：工具仅用于简单的网络测速，造成的一切后果自负**

</div>

## 📝 使用建议

- 建议从大范围 CIDR 中指定较大测速数量，并使用 `-tn` 参数
  - 例如：`-ip 2606:4700::/48=1000 -tn 300`
  - 含义是：对 2606:4700::/48 最多测速 1000 个随机 IP，并在延迟测速到 300 个可用 IP 后直接进行下一步
- 因为采取了流式处理，每个 IP 都实时生成、测速并过滤，内存中始终只有符合要求的结果

## 📊 测速结果示例

<img width="780" height="380" alt="演示图" src="https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/演示.png" />

> 这里 `x|y` 的含义是已进行下载测速 y 个，获取到 x 个符合要求的结果

## ✨ 功能特点

- 📊 下载测速期间，显示实时速度
- ⚡ IP 的生成和测速都是流式处理的，对 CIDR 依据采样数量均匀分割
- 🔌 优先使用指定端口测速，例如：`-ip [2606:4700::]:8080,104.16.0.0:80`
- 🌐 支持绑定到指定 IP 或接口名进行测速（`-intf`）
- ⏱️ 支持给程序限制运行时间，超时后立即结算结果并退出（`-timeout`）

## 🚀 示例命令

```bash
curl -s https://www.cloudflare-cn.com/ips-v4 -o ip.txt
```

```bash
-f ip.txt -tn 3000 -dn 10 -sl 15 -tlr 0 -httping https://cp.cloudflare.com/cdn-cgi/trace -url https://speed.cloudflare.com/__down?bytes=524288000
```

> [!IMPORTANT]
>- `speed.cloudflare.com` 不允许非 TLS 的 HTTP 下载测速，需自建测速地址
>- 慎用 IPv4 + HTTPSing 组合，可能会触发限制
>- 下载持续时间太短则不会算作有效速度，需确保下载测速文件足够大
>- 多网卡情况下，使用 -intf 参数会被策略路由影响效果
>- 注意队列数量和实时下载速度，设置合理的筛选条件
>- 可用 IP 数量是 Ping 通的，并非经历过筛选的数量
>- 如果不想写入文件，直接使用 -o 参数并不带值即可
>- 具体原理可参考 [流程图](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/main/Mermaid.md) 或 [时序图](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/main/时序图.md)

## 📋 参数说明

### 基本参数

| 参数 | 说明 | 示例 | 默认值 |
|:-----|:-----|:-------|:-------|
| `-url` | 下载测速所使用的测速地址 | https://example.com/file | 未指定 |
| `-f` | 从文件或文件路径读取 IP 或 CIDR | ip.txt | 未指定 |
| `-ip` | 直接指定 IP 或 CIDR（多个用逗号分隔） | 104.16.0.0/13=500,2606:4700::/36 | 未指定 |
| `-timeout` | 程序超时退出时间（秒） | 3600 | 不限制 |

### 测速参数

| 参数 | 说明 | 默认值 |
|:-----|:-----|:-------|
| `-t` | 延迟测速次数 | 4 |
| `-dn` | 下载测速所需符合要求的结果数量 | 10 |
| `-dt` | 下载测速时间（秒） | 10 |
| `-tp` | 测速端口 | 443 / 80 |
| `-all4` | 测速全部 IPv4 | 否 |
| `-tn` | 当 Ping 到指定可用数量，提前结束 Ping | 否 |

### 测速选项

| 参数 | 说明 | 示例 | 默认值 |
|:-----|:-----|:-------|:-------|
| `-httping` | 使用 HTTPing 测速并指定其地址 | N/A | http://cp.cloudflare.com/cdn-cgi/trace |
| `-dd` | 禁用下载测速 | N/A | 否 |
| `-hc` | 指定 HTTPing 的状态码 | 200,301,302 | 未指定 |
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

## 📥 下载链接

| 架构 \ 平台 | Linux | Linux_GNU | MacOS | Windows |
|:-----------|:------|:----------|:------|:--------|
| AMD64 | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Linux_AMD64/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Linux_AMD64_GNU/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/MacOS_AMD64/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Windows_AMD64/CloudflareST-Rust.exe) |
| ARM64 | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Linux_ARM64/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Linux_ARM64_GNU/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/MacOS_ARM64/CloudflareST-Rust) | [下载](https://gitee.com/zhxdcyy/CloudflareST-Rust/raw/master/binaries/Windows_ARM64/CloudflareST-Rust.exe) |

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
