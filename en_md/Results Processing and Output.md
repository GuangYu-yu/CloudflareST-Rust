## Results Processing and Output

Relevant source files

+   [src/common.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs)
+   [src/csv.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs)
+   [src/progress.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/progress.rs)

This document explains how CloudflareST-Rust processes test results and presents them to users. It covers the data structures used to store results, how results are filtered and sorted, and the different output formats available.

## 1\. Result Data Structures

The foundation of result processing is the `PingData` structure, which stores the outcome of individual network tests.

The main data structures:

1.  **`PingData`**: Core structure holding test results for a single IP address
2.  **`PingResult`**: Enum wrapping `PingData` to distinguish between test types
3.  **`PingDelaySet`**: Type alias for a vector of `PingData` (collection of results)

Sources: [src/common.rs16-45](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L16-L45)

## 2\. Result Processing Workflow

After network tests are completed, results go through several processing steps before being presented to the user:

The workflow includes:

1.  **Collection**: Results from multiple tests are gathered
2.  **Filtering**: Results are filtered based on user-defined criteria
3.  **Sorting**: Results are sorted by delay and loss rate
4.  **Presentation**: Formatted for display or export

Sources: [src/common.rs312-327](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L312-L327) [src/common.rs364-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L364-L374)

## 3\. Filtering Mechanisms

CloudflareST-Rust applies several filters to test results to ensure only relevant data is presented:

### 3.1 Basic Filtering

The `should_keep_result` function applies basic filtering criteria:

Sources: [src/common.rs312-327](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L312-L327)

### 3.2 Download Speed and Data Center Filtering

For download test results, additional filtering is applied through `process_download_result`:

Sources: [src/common.rs330-361](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L330-L361)

## 4\. Sorting Results

Once filtered, results are sorted to present the most relevant IPs first:

The `sort_ping_results` function orders results first by ping delay (lower is better) and then by packet loss rate (lower is better) when latencies are equal.

Sources: [src/common.rs364-374](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L364-L374)

## 5\. Output Formats

CloudflareST-Rust provides two primary output formats: console display and CSV export.

### 5.1 Console Output

Console output is implemented through the `PrintResult` trait:

The console output includes:

+   Formatted table with colored headers
+   Limited to the number of results specified by `args.print_num`
+   Different test types (HTTP, TCP, ICMP) are handled appropriately

Sources: [src/csv.rs50-105](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs#L50-L105)

### 5.2 CSV Export

Results can be exported to a CSV file for further analysis:

The CSV output includes the same data as the console display but in a format suitable for spreadsheet applications or other data processing tools.

Sources: [src/csv.rs14-48](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs#L14-L48)

## 6\. Data Formatting

The system uses specialized formatting functions to convert `PingData` into different output formats:

| Function | Purpose | Output Format |
| --- | --- | --- |
| `ping_data_to_csv_record` | Creates CSV records | Vector of strings |
| `ping_data_to_table_row` | Creates console table rows | `prettytable::Row` |
| `extract_data_center` | Extracts Cloudflare data center info | String |
| `calculate_precise_delay` | Calculates average delay with precision | Float (2 decimal places) |

Key formatting details:

+   IP addresses are displayed as strings
+   Loss rate is calculated and formatted to 2 decimal places
+   Delay is formatted to 2 decimal places
+   Download speed is converted from bytes/sec to MB/sec
+   Data center information is extracted from Cloudflare response headers

Sources: [src/common.rs243-272](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L243-L272) [src/common.rs156-175](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L156-L175) [src/common.rs69-78](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L69-L78)

## 7\. Progress Visualization

During testing, the system provides real-time feedback using the `Bar` component:

The progress visualization:

+   Adapts to terminal width
+   Shows completion percentage
+   Displays additional status information
+   Uses animation to indicate active processing

Sources: [src/progress.rs6-105](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/progress.rs#L6-L105)

## 8\. Integration with Testing Methods

Results processing is tightly integrated with the various testing methods:

Each testing method contributes results to a central collection, which is then processed through the filtering, sorting, and output stages as a unified workflow.

Sources: [src/common.rs221-240](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/common.rs#L221-L240) [src/csv.rs56-104](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs#L56-L104)

## 9\. Table Headers and Format

The system uses consistent headers for both console display and CSV export:

```text
IP 地址 | 已发送 | 已接收 | 丢包率 | 平均延迟 | 下载速度 (MB/s) | 数据中心
```

These headers represent:

1.  IP Address
2.  Packets Sent
3.  Packets Received
4.  Loss Rate
5.  Average Delay
6.  Download Speed (MB/s)
7.  Data Center

The console display uses the `prettytable` crate to create formatted tables with color-coded headers for better readability.

Sources: [src/csv.rs8-12](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs#L8-L12) [src/csv.rs73-79](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/csv.rs#L73-L79)

## Summary

The results processing system in CloudflareST-Rust provides a robust pipeline for handling test outcomes, from raw data collection to formatted presentation. It offers flexible filtering options to help users identify the best-performing Cloudflare IPs based on their specific requirements, with clear visual output both during and after testing.