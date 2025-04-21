# CloudflareST-Rust

**对 [XIU2/CloudflareSpeedTest](https://github.com/XIU2/CloudflareSpeedTest) 使用 Rust 重写**

***工具仅用于简单的网络测速，造成的一切后果自负***

> 可以先使用 [CIDR测速](https://github.com/GuangYu-yu/cfspeed) ，从延迟较低的 CIDR 中生成随机 IP

> [!TIP]
> - 使用自适应线程，最大 1024 线程
> - IP 的生成和测速都是流式处理的
> - 支持从指定 URL 中获取测速 CIDR/IP 列表
> - 支持从指定 URL 中获取测速地址列表
> - 使用 Httping 或下载测速时，显示数据中心
> - 支持给程序限制运行时间
> - 下载测速期间，显示实时速度

```
基本参数：
    -url
        测速地址 (https://example.com/file) [默认: 未指定]
    -urlist
        从 URL 内读取测速地址列表 (https://example.com/url_list.txt) [默认: 未指定]
    -f
        从文件或文件路径读取 IP 或 CIDR [默认: ip.txt]
    -ip
        直接指定 IP 或 CIDR (多个用逗号分隔) [默认: 未指定]
    -ipurl
        从URL读取 IP 或 CIDR (https://example.com/ip_list.txt) [默认: 未指定]
    -o
        输出结果文件（文件名或文件路径） [默认: result.csv]
    -h
        打印帮助说明 [默认: 否]
    -timeout
        程序超时退出时间（示例：1h3m6s） [默认: 不限制]
    
测速参数：
    -t
        延迟测速次数 [默认: 4]
    -dn
        所需下载测速结果数量 [默认: 10]
    -dt
        下载测速时间（秒） [默认: 10]
    -tp
        测速端口 [默认: 443]
    -dd
        禁用下载测速 [默认: 否]
    -all4
        测速全部IPv4 [默认: 否]
    
HTTP测速选项：
    -httping
        Httping模式 [默认: 否]
    -hc
        有效状态码 [默认: 接受200/301/302]
    -colo
        匹配指定地区（示例：HKG,SJC） [默认: 未指定]
    
筛选参数：
    -tl
        延迟上限（毫秒） [默认: 2000]
    -tll
        延迟下限（毫秒） [默认: 0]
    -tlr
        丢包率上限 [默认: 1.00]
    -sl
        下载速度下限（MB/s） [默认: 0.00]
    -p
        终端显示结果数量 [默认: 10]
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
