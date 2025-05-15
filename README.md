# CloudflareST-Rust

**对 [XIU2/CloudflareSpeedTest](https://github.com/XIU2/CloudflareSpeedTest) 使用 Rust 重写**

自建测速地址 [点击前往](https://github.com/GuangYu-yu/CF-Workers-SpeedTestURL)
> 测速地址搜寻语法 `title="Index of /" && server=="cloudflare" && port="80" && body=".iso"`

[![Ask GitSummarize](https://gitsummarize.com/favicon.ico)](https://gitsummarize.com/GuangYu-yu/CloudflareST-Rust) [![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust)

![时序图](https://gitee.com/zhxdcyy/cfurl/raw/master/4ce77970-3d26-4553-90e9-1d15c65da238.png)

***工具仅用于简单的网络测速，造成的一切后果自负***

> - 建议指定大范围 CIDR 较大测速数量，并使用 -tn 参数。例如：-ip 2606:4700::/48=100000 -tn 30000
> - 含义是：对 2606:4700::/48 最多测速 100000 个随机 IP ,并在测速到 30000 个可用 IP 后结束延迟测速
> - 因为是流式处理，因此即便测速任意数量随机 IP，都是实时生成、测速、过滤，内存中只有符合要求的结果

> [!TIP]
> - IP 的生成和测速都是流式处理的
> - 下载测速期间，显示实时速度
> - 优先使用指定端口测速，例如： `-ip [2606:4700::]:8080,104.16.0.0:80`
> - 支持从指定 URL 中获取测速 CIDR/IP 列表（-ipurl）
> - 支持从指定 URL 中获取测速地址列表（-urlist）
> - 使用了 Httping 或下载测速之后，会在结果显示数据中心
> - 支持给程序限制运行时间，超时后立即结算结果并退出
> - 当 IP 的 Ping 通数量达到 -tn 所指定的数量，则立即结算并开始执行下载测速
> - 使用 -httping 时，不带 TLS ，通过 `http://<IP>/cdn-cgi/trace` 进行测速
> - 使用 -hu 时，带 TLS 进行测速，可指定 Httping 测速地址
> - 使用 -hu 时，如果没有指定测速地址，则使用 -urlist 或 -url 提供的地址
> - 可对 CIDR 指定生成的随机 IP 数量，用于测速使用，例如： `2606:4700::/48=100000`

```
-ip 2606:4700:100::/48=120000,2606:4700:102::/48=120000 -tn 20000 -dn 20 -sl 18 -tl 200 -tlr 0 -url https://example.com
```

``` yaml
# CloudflareST-Rust

基本参数:
  -url         TLS 模式的 Httping 或下载测速所使用的测速地址（https://example.com/file）[默认：未指定]
  -urlist      从 URL 内读取测速地址列表（https://example.com/url_list.txt）[默认：未指定]
  -f           从文件或文件路径读取 IP 或 CIDR [默认：未指定]
  -ip          直接指定 IP 或 CIDR（多个用逗号分隔）[默认：未指定]
  -ipurl       从URL读取 IP 或 CIDR （https://example.com/ip_list.txt) [默认：未指定]
  -timeout     程序超时退出时间（秒）[默认：不限制]

测速参数:
  -t           延迟测速次数 [默认：4]
  -dn          所需下载测速结果数量 [默认：10]
  -dt          下载测速时间（秒）[默认：10]
  -tp          测速端口 [默认：443]
  -all4        测速全部 IPv4 [默认：否]
  -tn          当 Ping 到指定可用数量，提前结束 Ping [默认：否]

测速选项:
  -httping     使用非 TLS 模式的 Httping ，无需测速地址 [默认：否]
  -dd          禁用下载测速 [默认：否]
  -hu          使用 TLS 模式的 Httping ，可指定其 URL 测速地址或使用-url 或 -urlist 指定 [默认：否]
  -colo        匹配指定地区（示例：HKG,SJC）[默认：未指定]
  -n           延迟测速的线程数量 [默认：128]

结果参数:
  -tl          延迟上限（毫秒）[默认：2000]
  -tll         延迟下限（毫秒）[默认：0]
  -tlr         丢包率上限 [默认：1.00]
  -sl          下载速度下限（MB/s）[默认：0.00]
  -p           终端显示结果数量 [默认：10]
  -sp          启用结果的端口号显示 [默认：否]
  -o           输出结果文件（文件名或文件路径）[默认：result.csv]
```

``` yaml
# CloudflareST-Rust

开始延迟测速（模式：Tcping, 端口：443, 范围：0 ~ 300 ms, 丢包：0.20)
30000/30000 [==========================================↖] 可用：10811
开始下载测速（下限：15.00 MB/s, 所需：10, 队列：10811）
10/10 [=================================================↘] 15.58 MB/s
IP 地址           已发送  已接收  丢包率    平均延迟    下载速度 (MB/s)   数据中心
104.25.---.--     8       8       0.00      65.72       20.18              LAX
104.18.---.---    8       8       0.00      99.94       19.36              SJC
104.25.---.---    8       8       0.00      84.28       19.08              LAX
104.25.---.--     8       8       0.00      98.01       18.91              LAX
172.64.---.--     8       7       0.12     100.44       18.47              LAX
104.25.---.--     8       8       0.00      95.47       18.44              FRA
104.25.---.---    8       8       0.00      97.92       18.35              LAX
104.25.---.---    8       7       0.12     100.66       18.29              LAX
104.25.---.--     8       7       0.12     100.47       18.21              FRA
104.25.---.---    8       8       0.00      99.70       18.18              FRA

[信息] 完整测速结果已写入 result.csv 文件，可使用记事本/表格软件查看
程序执行完毕
```

## 下载直链

[linux_amd64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/linux_amd64/CloudflareST-Rust)

[linux_arm64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/linux_arm64/CloudflareST-Rust)

[macos_x86_64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/macos_x86_64/CloudflareST-Rust)

[macos_arm64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/macos_arm64/CloudflareST-Rust)

[windows_x86_64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/windows_x86_64/CloudflareST-Rust.exe)

[windows_arm64](https://raw.githubusercontent.com/GuangYu-yu/CloudflareST-Rust/refs/heads/main/binaries/windows_arm64/CloudflareST-Rust.exe)

****

## License

The GPL-3.0 License.
