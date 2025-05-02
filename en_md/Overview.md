## Overview

Relevant source files

+   [binaries/linux\_amd64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/linux_amd64/CloudflareST-Rust)
+   [binaries/macos\_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_arm64/CloudflareST-Rust)
+   [binaries/windows\_x86\_64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_x86_64/CloudflareST-Rust.exe)
+   [src/main.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs)

CloudflareST-Rust is a network performance testing tool designed to evaluate and optimize connections to Cloudflare's infrastructure. This document provides a comprehensive overview of the system architecture, core components, and operational workflow of CloudflareST-Rust. For detailed information about installation and usage, see [Installation and Usage](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/2-installation-and-usage).

## Purpose and Capabilities

CloudflareST-Rust evaluates network connectivity to Cloudflare's content delivery network (CDN) through multiple testing methodologies, including TCP, HTTP, and ICMP ping tests, followed by optional download speed measurements. The tool helps users identify optimal Cloudflare edge servers (colos) based on metrics such as latency, packet loss, and throughput.

Key capabilities include:

+   **Multiple Testing Methods**: TCP, HTTP, and ICMP ping testing with configurable parameters
+   **Download Speed Testing**: Measurement of real-world throughput to selected servers
+   **IP Filtering**: Advanced filtering based on latency, loss rate, and other metrics
+   **Result Sorting**: Sorting and selection of optimal servers based on user-defined criteria
+   **Cross-Platform Support**: Native performance on Windows, macOS, and Linux

Sources: [src/main.rs1-15](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L1-L15) [src/main.rs42-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L42-L57)

## System Architecture

CloudflareST-Rust employs a modular architecture with distinct components for different aspects of network testing. The core of the system is built around a concurrent testing framework that efficiently manages network operations.

Sources: [src/main.rs1-10](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L1-L10) [src/main.rs42-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L42-L57)

## Testing Workflow

CloudflareST-Rust follows a systematic workflow from initialization to result output. The application processes command-line arguments, selects the appropriate testing methodology, performs tests, and outputs results.

Sources: [src/main.rs20-85](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L20-L85)

## Data Structures

CloudflareST-Rust uses several key data structures to manage test data and results. The most important is the `PingResult` enum, which encapsulates the results of different types of ping tests.

Sources: [src/main.rs93-99](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L93-L99) [src/common.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs)

## Thread Pool and Concurrency

CloudflareST-Rust employs a sophisticated thread pool to manage concurrent network operations efficiently. The thread pool dynamically adjusts the number of concurrent operations based on system capabilities and network conditions.

Sources: [src/pool.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/pool.rs)

## Cross-Platform Support

CloudflareST-Rust is a cross-platform application compiled for multiple architectures to provide native performance across different operating systems. The table below summarizes the supported platforms and architectures:

| Operating System | Architectures | Binary Location |
| --- | --- | --- |
| macOS | ARM64 (Apple Silicon), x86\_64 (Intel) | `binaries/macos_arm64/`, `binaries/macos_x86_64/` |
| Windows | ARM64, x86\_64 | `binaries/windows_arm64/`, `binaries/windows_x86_64/` |
| Linux | ARM64, AMD64 (x86\_64) | `binaries/linux_arm64/`, `binaries/linux_amd64/` |

The application maintains consistent functionality across all platforms while taking advantage of platform-specific optimizations where possible.

Sources: [binaries/macos\_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_arm64/CloudflareST-Rust) [binaries/windows\_x86\_64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_x86_64/CloudflareST-Rust.exe)

## Execution Flow

The execution flow of CloudflareST-Rust begins with the main function, which coordinates all aspects of the application's operation.

Sources: [src/main.rs20-85](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L20-L85)

## Network Testing Methods

CloudflareST-Rust implements three primary methods for network testing, each with unique characteristics and use cases. For detailed information about each testing method, see [Network Testing Methods](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4-network-testing-methods).

| Test Method | Module | Description | Default Port/Protocol |
| --- | --- | --- | --- |
| TCP Ping | `tcping.rs` | Tests TCP connection establishment time | Configurable (default: 443) |
| HTTP Ping | `httping.rs` | Tests HTTP request-response time | HTTP(S) on port 443 |
| ICMP Ping | `icmp.rs` | Uses ICMP echo request/reply | ICMP protocol |
| Download | `download.rs` | Measures actual download speed | HTTP(S) on port 443 |

The appropriate test method is selected based on command-line arguments, with TCP ping being the default if no specific method is specified.

Sources: [src/main.rs42-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L42-L57) [src/tcping.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs) [src/httping.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/httping.rs) [src/icmp.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/icmp.rs) [src/download.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs)