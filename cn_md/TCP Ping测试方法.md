## TCP Ping测试方法

相关源文件

+   [src/common.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs)
+   [src/tcping.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs)

本文档记录了CloudflareST-Rust中的TCP Ping测试功能，该功能通过TCP连接尝试测量与Cloudflare服务器的连接延迟和数据包丢失率。其他测试方法请参见[HTTP Ping测试](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.1-http-ping-testing)或[ICMP Ping测试](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.3-icmp-ping-testing)。

## 概述

TCP Ping(TCPing)通过测量与目标服务器建立TCP连接所需的时间来衡量网络延迟。与发送完整HTTP请求的HTTP Ping测试不同，TCP Ping仅建立连接而不发送应用数据，使其更轻量、更快，同时仍能反映真实世界的TCP连接性能。

来源: [src/tcping.rs15-139](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L139) [src/common.rs15-46](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L46) [src/common.rs362-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L362-L374)

## 实现细节

### 核心组件

TCP Ping实现包含三个主要函数:

1.  `Ping::run()` - 管理测试工作流程的主协调函数
2.  `tcping_handler()` - 处理特定IP地址的测试
3.  `tcping()` - 执行单个TCP连接尝试

来源: [src/tcping.rs15-39](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L39) [src/tcping.rs141-226](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L141-L226) [src/common.rs15-46](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L46)

### Ping结构体

`Ping`结构体管理整个TCP ping测试过程，包含以下关键组件:

| 字段 | 类型 | 用途 |
| --- | --- | --- |
| `ip_buffer` | `Arc<Mutex<IpBuffer>>` | 线程安全的待测试IP地址缓冲区 |
| `csv` | `Arc<Mutex<PingDelaySet>>` | 线程安全的ping结果集合 |
| `bar` | `Arc<Bar>` | 进度条提供视觉反馈 |
| `max_loss_rate` | `f32` | 用于过滤的最大可接受丢包率 |
| `args` | `Args` | 命令行参数和配置 |
| `success_count` | `Arc<AtomicUsize>` | 成功ping测试计数 |
| `timeout_flag` | `Arc<AtomicBool>` | 停止测试的信号标志 |

来源: [src/tcping.rs15-23](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L23)

## 测试流程

### 初始化

TCP Ping测试从初始化测试环境开始:

1.  从提供的源创建IP缓冲区
2.  设置带有预期IP总数的进度条
3.  初始化存储结果的容器
4.  设置过滤参数(最小/最大延迟，最大丢包率)

来源: [src/tcping.rs25-38](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L25-L38) [src/common.rs220-240](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L220-L240)

### 执行流程

TCP Ping测试流程遵循以下步骤:

1.  检查IP缓冲区是否有待测试IP
2.  显示测试信息(端口、延迟范围、丢包率阈值)
3.  使用动态线程池设置并发测试
4.  对每个IP地址:
    +   生成任务处理TCP ping测试
    +   收集并处理结果
5.  处理结果并按延迟排序

来源: [src/tcping.rs40-138](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L138)

### TCP连接测试

核心TCP ping功能在`tcping`函数中实现，该函数:

1.  创建与目标IP和端口的TCP连接
2.  测量建立连接所需时间
3.  如果成功则返回以毫秒为单位的连接时间，失败则返回`None`

来源: [src/tcping.rs191-226](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L191-L226)

### 并发管理

TCP Ping实现使用动态线程池并发测试多个IP地址:

1.  基于可用CPU核心启动初始任务批次
2.  任务完成后，生成新任务保持池忙碌
3.  使用`FuturesUnordered`管理异步任务
4.  实现速率限制防止网络过载

线程池根据系统性能和负载自动调整并发级别。

来源: [src/tcping.rs66-126](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L66-L126)

## 数据结构

### PingData结构

TCP ping测试结果存储在`PingData`结构中:

| 字段 | 类型 | 描述 |
| --- | --- | --- |
| `ip` | `IpAddr` | 被测试的IP地址 |
| `sent` | `u16` | 连接尝试次数 |
| `received` | `u16` | 成功连接次数 |
| `delay` | `f32` | 平均连接时间(毫秒) |
| `download_speed` | `Option<f32>` | 可选下载速度(如果测试) |
| `data_center` | `String` | Cloudflare数据中心标识符 |

`loss_rate()`方法计算丢包率为`1.0 - (received / sent)`。

来源: [src/common.rs15-43](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L43)

## 结果处理

所有ping测试完成后，结果将:

1.  基于配置标准进行过滤:
    +   最小和最大延迟阈值
    +   最大可接受丢包率
2.  主要按延迟排序(越低越好)
3.  作为`PingDelaySet`(`PingData`向量)返回

来源: [src/common.rs312-327](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L312-L327) [src/common.rs362-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L362-L374) [src/tcping.rs132-137](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L132-L137)

## 与其他组件的集成

TCP Ping测试与其他系统组件集成:

1.  使用全局线程池进行并发执行
2.  使用进度条显示测试进度
3.  将结果提供给CSV导出器进行报告
4.  可选地将结果提供给下载测试模块

TCP Ping模块设计为可独立使用，也可作为包含下载速度测试的更大测试工作流的一部分。

来源: [src/tcping.rs11-13](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L11-L13) [src/tcping.rs40-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L57)

## 使用示例流程

当选择TCP Ping测试时(默认或通过命令行参数)，应用程序:

1.  使用适当参数创建新的`Ping`实例
2.  调用`run()`方法对所有IP执行TCP ping测试
3.  处理并显示结果
4.  可选地使用过滤后的IP集继续下载测试

此测试通常从主程序流启动，然后使用结果通知下游测试或决策。

来源: [src/tcping.rs25-39](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L25-L39) [src/tcping.rs40-139](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L139)