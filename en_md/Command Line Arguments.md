## Command Line Arguments

Relevant source files

+   [src/args.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs)
+   [src/main.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs)

This document provides a comprehensive reference of all command line arguments available in CloudflareST-Rust, explaining their meaning, default values, and how they affect program behavior. For details on installation and basic usage, see [Installation and Usage](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/2-installation-and-usage).

## Argument Processing Workflow

When CloudflareST-Rust starts, it processes command line arguments through several steps:

1.  Arguments are collected from the command line using `std::env::args()`
2.  Arguments are parsed and converted to appropriate types in the `Args` struct
3.  Validation checks ensure required arguments are provided
4.  Default values are applied for unspecified arguments
5.  The final `Args` struct is used throughout the application

Sources: [src/args.rs316-346](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L316-L346) [src/main.rs22-23](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L22-L23)

## Argument Structure

The command line arguments are defined in the `Args` struct, which organizes them into logical categories:

Sources: [src/args.rs6-48](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L6-L48)

## Argument Categories

CloudflareST-Rust's command line arguments are organized into five major categories:

### 1\. Basic Configuration Arguments

These arguments define basic program operation, including IP sources and testing URLs:

| Argument | Description | Default |
| --- | --- | --- |
| `-url` | Testing URL for HTTP ping and download tests | None (required for HTTP/download tests) |
| `-urlist` | URL to a file containing list of testing URLs | None |
| `-f` | Path to a file containing IP addresses or CIDR blocks | None |
| `-ip` | Directly specify IPs or CIDR blocks (comma-separated) | None |
| `-ipurl` | URL to a file containing IP addresses or CIDR blocks | None |
| `-h` | Show help information | `false` |
| `-timeout` | Global timeout for the program (e.g., "1h3m6s") | None (no limit) |

**Note:** At least one IP source (`-f`, `-ip`, or `-ipurl`) must be specified.

Sources: [src/args.rs323-329](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L323-L329) [src/args.rs277-286](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L277-L286)

### 2\. Network Testing Configuration

These arguments control the network testing parameters:

| Argument | Description | Default |
| --- | --- | --- |
| `-t` | Number of ping tests per IP | `4` |
| `-dn` | Number of download test results to collect | `10` |
| `-dt` | Download test duration in seconds | `10` |
| `-tp` | Port to use for TCP testing | `443` |
| `-dd` | Disable download testing | `false` |
| `-all4` | Test all IPv4 addresses in range | `false` |
| `-tn` | Stop pinging after finding this many suitable IPs | None |

Sources: [src/args.rs288-296](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L288-L296)

### 3\. Test Mode Selection

These arguments determine which testing methods will be used:

| Argument | Description | Default |
| --- | --- | --- |
| `-httping` | Use HTTP ping testing mode | `false` |
| `-ping` | Use ICMP ping testing mode | `false` |
| `-hc` | Valid HTTP status codes for HTTP ping tests | None (accepts 200/301/302) |
| `-hu` | Specify URLs for HTTP ping testing (comma-separated) | None |
| `-colo` | Filter by Cloudflare datacenter location (e.g., "HKG,SJC") | None |
| `-n` | Maximum thread count for the dynamic thread pool | `1024` |

**Note:** If neither `-httping` nor `-ping` is specified, TCP ping mode is used by default.

Sources: [src/args.rs298-305](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L298-L305) [src/main.rs44-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L44-L57)

### 4\. Results Filtering

These arguments set thresholds for filtering test results:

| Argument | Description | Default |
| --- | --- | --- |
| `-tl` | Maximum acceptable delay (milliseconds) | `2000` |
| `-tll` | Minimum acceptable delay (milliseconds) | `0` |
| `-tlr` | Maximum acceptable packet loss rate (0.0-1.0) | `1.0` |
| `-sl` | Minimum acceptable download speed (MB/s) | `0.0` |

Sources: [src/args.rs308-311](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L308-L311)

### 5\. Output Configuration

These arguments control how results are displayed and saved:

| Argument | Description | Default |
| --- | --- | --- |
| `-p` | Number of results to display in terminal | `10` |
| `-o` | Output CSV file path | `result.csv` |

Sources: [src/args.rs312-313](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L312-L313)

## Arguments Parsing Logic

CloudflareST-Rust parses command line arguments in a flexible way, supporting both single (`-arg`) and double-dash (`--arg`) prefixes. The argument parsing logic handles both flag arguments (without values) and arguments with values.

Sources: [src/args.rs90-240](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L90-L240)

## Validation and Default Values

Some arguments require additional validation:

1.  **IP Source Requirement**: At least one IP source (`-f`, `-ipurl`, or `-ip`) must be specified
2.  **HTTP Test URL Requirement**: When using `-httping`, a URL must be specified via `-hu`, `-url`, or `-urlist`
3.  **Download Test URL Requirement**: When download testing is enabled (no `-dd`), a URL must be specified via `-url` or `-urlist`

Default values are set in the `Args::new()` method, ensuring that even when arguments aren't specified, the program has sensible defaults.

Sources: [src/args.rs51-88](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L51-L88) [src/args.rs323-343](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L323-L343)

## Duration Parsing

CloudflareST-Rust uses a special parser for duration-based arguments (`-timeout`, `-dt`) that supports multiple formats:

1.  Plain numbers (interpreted as seconds)
2.  Human-readable durations (e.g., "1h30m", "5m10s")

Sources: [src/args.rs244-266](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L244-L266)

## Common Argument Combinations

Different testing scenarios typically use different combinations of arguments:

### TCP Ping Testing (Default)

Basic TCP connectivity and latency testing:

```text
CloudflareST-Rust -f ip.txt -o results.csv
```

### HTTP Ping Testing

Testing HTTP response times for web services:

```text
CloudflareST-Rust -f ip.txt -httping -url https://example.com
```

### ICMP Ping Testing

Classic ping test using ICMP packets:

```text
CloudflareST-Rust -f ip.txt -ping
```

### Full Testing with Filters

Comprehensive testing with filter criteria:

```text
CloudflareST-Rust -f ip.txt -url https://example.com -tl 100 -tlr 0.05 -sl 10 -t 10
```

Sources: [src/main.rs44-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L44-L57)

## Command Line Help

The application provides a comprehensive help listing via the `-h` argument, which displays all available arguments organized by category. This is implemented in the `print_help()` function.

Sources: [src/args.rs274-314](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L274-L314)

## Global Timeout Mechanism

The `global_timeout` argument sets a maximum execution time for the entire program. When specified, CloudflareST-Rust will:

1.  Parse the duration string into a `Duration` value
2.  Create an atomic boolean flag (`timeout_flag`)
3.  Spawn a background thread that will set the flag after the timeout period
4.  Check this flag at key points in the program flow to allow early termination

Sources: [src/args.rs219-222](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs#L219-L222) [src/main.rs26-36](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L26-L36) [src/main.rs60-68](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L60-L68)

## Relationship with Other Components

The `Args` struct and parsed arguments are passed to various components throughout the application:

Sources: [src/main.rs44-57](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L44-L57) [src/main.rs70](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L70-L70) [src/main.rs77](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/main.rs#L77-L77)

## Conclusion

The command line argument system in CloudflareST-Rust provides a flexible and powerful interface for controlling the application's behavior. Through a combination of required and optional arguments, users can precisely tailor the testing process to their specific needs while relying on sensible defaults for unspecified parameters.