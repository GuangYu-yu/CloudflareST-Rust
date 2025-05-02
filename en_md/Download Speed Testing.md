## Download Speed Testing

Relevant source files

+   [src/common.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs)
+   [src/download.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs)

This document details the download speed testing component of CloudflareST-Rust, which evaluates the bandwidth performance of Cloudflare CDN endpoints. The download speed testing occurs after initial ping tests (HTTP, TCP, or ICMP) and is designed to measure real-world throughput from selected IP addresses. This page focuses specifically on the implementation and behavior of the download speed measurement functionality.

For information about the initial ping testing methods that precede download testing, see [HTTP Ping Testing](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.1-http-ping-testing), [TCP Ping Testing](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.2-tcp-ping-testing), and [ICMP Ping Testing](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.3-icmp-ping-testing).

## Overview

The download speed testing module evaluates the actual data transfer rate achievable from Cloudflare servers. It uses a sampling approach to measure throughput accurately while limiting test duration. The system supports:

+   Configurable minimum speed thresholds
+   Multiple URL testing
+   Data center (colo) filtering
+   EWMA (Exponentially Weighted Moving Average) for stable measurements
+   Integration with the progress display subsystem

Sources: [src/download.rs149-182](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L149-L182) [src/download.rs335-453](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L335-L453)

## Architecture and Integration

Download testing is an optional phase that runs after the initial ping tests complete. It takes the best-performing IPs from the ping tests and measures their download performance.

Sources: [src/download.rs149-182](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L149-L182) [src/download.rs216-333](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L216-L333)

## Key Components

### DownloadTest Struct

The central component for download testing is the `DownloadTest` struct, which manages the entire testing process:

Sources: [src/download.rs149-216](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L149-L216) [src/download.rs47-147](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L47-L147) [src/download.rs17-44](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L17-L44)

## Download Speed Measurement Process

The download testing follows a precise sequence of operations:

1.  Initialize `DownloadTest` with ping results and configuration parameters
2.  For each IP in the filtered ping results:
    +   Connect to the target URL using the IP
    +   Download data for a configured duration
    +   Measure throughput using EWMA calculations
    +   Record and display real-time speeds
3.  Filter results based on minimum speed and colo requirements
4.  Sort results by download speed, latency, and loss rate

Sources: [src/download.rs216-333](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L216-L333) [src/download.rs335-453](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L335-L453)

## Speed Calculation Methodology

The download speed measurement uses several techniques to ensure accurate readings:

### 1\. Moving Window Sampling

The system maintains a rolling 500ms window of data points to calculate the current speed:

```text
speed_samples: VecDeque<(Instant, u64)>
```

When new data arrives, it:

+   Adds the current timestamp and accumulated data to the queue
+   Removes samples older than 500ms
+   Calculates speed based on first and last samples in the window

Sources: [src/download.rs83-113](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L83-L113)

### 2\. Exponentially Weighted Moving Average (EWMA)

To smooth out speed fluctuations, the system implements EWMA:

```text
ewma.add(content_diff as f32)
```

This gives more weight to recent measurements while still considering historical data, resulting in more stable readings.

Sources: [src/download.rs17-44](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L17-L44) [src/download.rs123-137](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L123-L137)

### 3\. Time Slice Processing

The speed measurement is divided into 100ms time slices:

```text
time_slice: Duration::from_millis(100)
```

Each slice contributes to the EWMA calculation, helping to even out network jitter and provide consistent measurements.

Sources: [src/download.rs70-71](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L70-L71) [src/download.rs123-137](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L123-L137)

## Result Processing and Filtering

After download testing, results are processed based on several criteria:

1.  **Minimum Speed**: Results must exceed the configured minimum speed (in MB/s)
2.  **Data Center (Colo) Filtering**: Optional filtering by Cloudflare data center identifier
3.  **Sorting**: Results are sorted by:
    +   Download speed (descending)
    +   Ping delay (ascending)
    +   Loss rate (ascending)

Sources: [src/download.rs166-182](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L166-L182) [src/common.rs328-361](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L328-L361)

## Data Structures

### PingData

Results from both ping and download tests are stored in the `PingData` structure:

| Field | Type | Description |
| --- | --- | --- |
| ip | IpAddr | IP address being tested |
| sent | u16 | Number of packets sent |
| received | u16 | Number of packets received |
| delay | f32 | Average ping delay in ms |
| download\_speed | Option | Download speed in bytes/sec (if tested) |
| data\_center | String | Cloudflare data center identifier |

The download testing phase specifically updates the `download_speed` and (if needed) `data_center` fields.

Sources: [src/common.rs16-43](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L16-L43)

## Configuration Parameters

The download testing behavior can be customized with several command-line parameters:

| Parameter | Description | Default |
| --- | --- | --- |
| `--disable-download` | Skip download testing | false |
| `--url` | URL for download testing | `http://1.1.1.1/cdn-cgi/trace` |
| `--urlist` | URL pointing to a list of test URLs | empty |
| `--min-speed` | Minimum required speed in MB/s | 0.0 |
| `--test-count` | Number of IPs to test for download | 10 |
| `--httping-cf-colo` | Filter by Cloudflare data center codes | empty |

Sources: [src/download.rs184-216](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L184-L216) [src/common.rs328-361](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L328-L361)

## Real-time Progress Display

The download testing component integrates with the progress display subsystem to show real-time speed information:

1.  A dedicated task updates the progress bar with current speed (MB/s)
2.  The current speed is stored in a thread-safe `Arc<Mutex<f32>>`
3.  Updates occur approximately every 500ms for a balance of responsiveness and stability

Sources: [src/download.rs232-249](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L232-L249) [src/download.rs96-120](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L96-L120)

## Error Handling and Timeout Management

The download testing system implements several error handling mechanisms:

1.  **Timeout Detection**: Tests are limited by a configurable duration
2.  **Global Timeout Flag**: An `AtomicBool` allows graceful interruption
3.  **Connection Failures**: Failed connections result in zero speed and are filtered out
4.  **HTTP Error Handling**: Failed HTTP responses are properly handled

Sources: [src/download.rs394-407](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L394-L407) [src/download.rs253-255](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L253-L255)

## Implementation Notes

1.  The implementation prioritizes accurate measurement over maximizing throughput
2.  Small chunks of data are continually requested rather than a single large file
3.  Special handling for partial time slices ensures accurate measurements
4.  The system is designed to work with Cloudflare's CDN architecture and reports data center information

Sources: [src/download.rs409-434](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L409-L434) [src/download.rs383-387](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/download.rs#L383-L387)