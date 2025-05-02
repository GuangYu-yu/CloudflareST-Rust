## TCP Ping Testing

Relevant source files

+   [src/common.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs)
+   [src/tcping.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs)

This page documents the TCP Ping testing functionality in CloudflareST-Rust, which measures connection latency and packet loss to Cloudflare servers using TCP connection attempts. For other testing methods, see [HTTP Ping Testing](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.1-http-ping-testing) or [ICMP Ping Testing](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/4.3-icmp-ping-testing).

## Overview

TCP Ping (TCPing) measures network latency by timing how long it takes to establish a TCP connection to a target server. Unlike HTTP Ping testing which sends complete HTTP requests, TCP Ping only establishes the connection without sending application data, making it lighter and faster but still reflecting real-world TCP connection performance.

Sources: [src/tcping.rs15-139](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L139) [src/common.rs15-46](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L46) [src/common.rs362-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L362-L374)

## Implementation Details

### Core Components

The TCP Ping implementation consists of three primary functions:

1.  `Ping::run()` - Main orchestration function that manages the testing workflow
2.  `tcping_handler()` - Handles testing for a specific IP address
3.  `tcping()` - Performs individual TCP connection attempts

Sources: [src/tcping.rs15-39](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L39) [src/tcping.rs141-226](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L141-L226) [src/common.rs15-46](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L46)

### The Ping Struct

The `Ping` struct manages the overall TCP ping testing process and contains the following key components:

| Field | Type | Purpose |
| --- | --- | --- |
| `ip_buffer` | `Arc<Mutex<IpBuffer>>` | Thread-safe buffer of IP addresses to test |
| `csv` | `Arc<Mutex<PingDelaySet>>` | Thread-safe collection of ping results |
| `bar` | `Arc<Bar>` | Progress bar for visual feedback |
| `max_loss_rate` | `f32` | Maximum acceptable packet loss rate for filtering |
| `args` | `Args` | Command line arguments and configuration |
| `success_count` | `Arc<AtomicUsize>` | Count of successful ping tests |
| `timeout_flag` | `Arc<AtomicBool>` | Flag to signal when testing should stop |

Sources: [src/tcping.rs15-23](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L15-L23)

## Testing Process

### Initialization

TCP Ping testing begins by initializing the testing environment:

1.  Creates an IP buffer from the provided sources
2.  Sets up a progress bar with the expected total IPs
3.  Initializes a container for storing results
4.  Sets filtering parameters (min/max delay, max loss rate)

Sources: [src/tcping.rs25-38](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L25-L38) [src/common.rs220-240](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L220-L240)

### Execution Flow

The TCP Ping testing process follows these steps:

1.  Check if the IP buffer has IPs to test
2.  Display test information (port, delay range, loss rate threshold)
3.  Set up concurrent testing with a dynamic thread pool
4.  For each IP address:
    +   Spawn a task to handle TCP ping testing
    +   Collect and process the results
5.  Process results and sort them by latency

Sources: [src/tcping.rs40-138](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L138)

### TCP Connection Testing

The core TCP ping functionality is implemented in the `tcping` function, which:

1.  Creates a TCP connection to the target IP and port
2.  Measures the time taken to establish the connection
3.  Returns the connection time in milliseconds if successful, or `None` if the connection fails

Sources: [src/tcping.rs191-226](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L191-L226)

### Concurrency Management

The TCP Ping implementation uses a dynamic thread pool to concurrently test multiple IP addresses:

1.  Starts with an initial batch of tasks based on available CPU cores
2.  As tasks complete, new tasks are spawned to keep the pool busy
3.  Uses `FuturesUnordered` to manage asynchronous tasks
4.  Implements rate limiting to prevent network overload

The thread pool adjusts the concurrency level automatically based on system performance and load.

Sources: [src/tcping.rs66-126](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L66-L126)

## Data Structures

### PingData Structure

The results of TCP ping tests are stored in the `PingData` structure:

| Field | Type | Description |
| --- | --- | --- |
| `ip` | `IpAddr` | IP address being tested |
| `sent` | `u16` | Number of connection attempts |
| `received` | `u16` | Number of successful connections |
| `delay` | `f32` | Average connection time in milliseconds |
| `download_speed` | `Option<f32>` | Optional download speed (if tested) |
| `data_center` | `String` | Cloudflare data center identifier |

The `loss_rate()` method calculates packet loss as `1.0 - (received / sent)`.

Sources: [src/common.rs15-43](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L15-L43)

## Result Processing

After all ping tests are completed, the results are:

1.  Filtered based on configured criteria:
    +   Minimum and maximum delay thresholds
    +   Maximum acceptable packet loss rate
2.  Sorted primarily by latency (lower is better)
3.  Returned as a `PingDelaySet` (vector of `PingData`)

Sources: [src/common.rs312-327](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L312-L327) [src/common.rs362-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L362-L374) [src/tcping.rs132-137](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L132-L137)

## Integration with Other Components

TCP Ping testing integrates with other system components:

1.  Uses the global thread pool for concurrent execution
2.  Uses the progress bar to display test progress
3.  Feeds results to the CSV exporter for reporting
4.  Optionally feeds results to the download testing module

The TCP Ping module is designed to be used either independently or as part of a larger testing workflow that includes download speed tests.

Sources: [src/tcping.rs11-13](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L11-L13) [src/tcping.rs40-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L57)

## Example Usage Flow

When TCP Ping testing is selected (by default or via command-line arguments), the application:

1.  Creates a new `Ping` instance with appropriate parameters
2.  Calls the `run()` method to execute TCP ping tests against all IPs
3.  Processes and displays the results
4.  Optionally proceeds to download testing with the filtered IP set

This testing is typically initiated from the main program flow, which then uses the results to inform downstream testing or decision-making.

Sources: [src/tcping.rs25-39](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L25-L39) [src/tcping.rs40-139](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/tcping.rs#L40-L139)