## Installation and Usage

Relevant source files

+   [binaries/linux\_amd64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/linux_amd64/CloudflareST-Rust)
+   [binaries/linux\_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/linux_arm64/CloudflareST-Rust)
+   [binaries/macos\_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_arm64/CloudflareST-Rust)
+   [binaries/macos\_x86\_64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_x86_64/CloudflareST-Rust)
+   [binaries/windows\_arm64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_arm64/CloudflareST-Rust.exe)
+   [binaries/windows\_x86\_64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_x86_64/CloudflareST-Rust.exe)
+   [src/args.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs)

This page provides comprehensive instructions for installing and using CloudflareST-Rust, a network performance testing tool designed to evaluate and optimize connections to Cloudflare's infrastructure. For an overview of the project and its capabilities, see [Overview](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/1-overview).

## Installation Options

CloudflareST-Rust is available as pre-compiled binaries for multiple platforms, making installation straightforward for most users. You can also build the application from source if needed.

### Pre-compiled Binaries

#### Diagram: CloudflareST-Rust Binary Architecture

Sources: `binaries/macos_arm64/CloudflareST-Rust`, `binaries/macos_x86_64/CloudflareST-Rust`, `binaries/windows_x86_64/CloudflareST-Rust.exe`, `binaries/windows_arm64/CloudflareST-Rust.exe`, `binaries/linux_amd64/CloudflareST-Rust`, `binaries/linux_arm64/CloudflareST-Rust`

#### Platform-specific Installation

1.  **Windows**:
    
    +   Download the appropriate binary (`CloudflareST-Rust.exe`) for your architecture (x86\_64 or ARM64)
    +   No installation required - the executable can be run directly
    +   Optionally, add the executable to your PATH for easier access
2.  **macOS**:
    
    +   Download the appropriate binary for your architecture (Intel or Apple Silicon)
    +   Open Terminal and navigate to the download location
    +   Make the binary executable: `chmod +x CloudflareST-Rust`
    +   Optionally, move to a location in your PATH: `sudo mv CloudflareST-Rust /usr/local/bin/`
3.  **Linux**:
    
    +   Download the appropriate binary for your architecture (AMD64 or ARM64)
    +   Make the binary executable: `chmod +x CloudflareST-Rust`
    +   Optionally, move to a location in your PATH: `sudo mv CloudflareST-Rust /usr/local/bin/`

### Building from Source

To build from source, you'll need:

+   Rust toolchain (rustc, cargo)
+   Git

1.  Clone the repository:
    
2.  Build the project:
    
3.  The compiled binary will be available in `target/release/`
    

## Basic Usage

CloudflareST-Rust is a command-line tool that evaluates network performance to Cloudflare infrastructure.

### Diagram: Basic Usage Flow

Sources: Binary files from various platforms

### Basic Command Structure

```text
CloudflareST-Rust [OPTIONS]
```
    

## Command-Line Arguments

CloudflareST-Rust provides numerous command-line arguments to customize testing behavior.

### Diagram: Command-Line Argument Categories

### Test Method Options

| Option | Description | Default |
| --- | --- | --- |
| `-http` | Use HTTP ping method | Disabled |
| `tcp` | Use TCP ping method | Enabled by default |
| `-icmp` | Use ICMP ping method | Disabled |

### IP Source Options

| Option | Description | Default |
| --- | --- | --- |
| `-f, --file <file>` | IP address file | Built-in Cloudflare IP list |
| `-url <address>` | URL to download IP list | None |

### Filtering Options

| Option | Description | Default |
| --- | --- | --- |
| `-sl, --speed-limit <ms>` | Minimum acceptable delay | No limit |
| `-tl, --time-limit <ms>` | Maximum acceptable delay | No limit |
| `-p, --max-loss <percent>` | Maximum acceptable packet loss rate | No limit |

### Output Options

| Option | Description | Default |
| --- | --- | --- |
| `-dd, --disable-download` | Skip download speed test | Download test enabled |
| `-o, --output <filename>` | CSV output file | No CSV export |

### Test Parameters

| Option | Description | Default |
| --- | --- | --- |
| `-n, --num-ping <count>` | Number of pings per IP | 4 |
| `-tp, --tcp-port <port>` | TCP port for testing | 443 |
| `-tn, --target-num <number>` | Number of IPs to display | All IPs |

## Usage Workflows

This section demonstrates complete workflows for common testing scenarios.

### Diagram: Complete Testing Workflow

### Basic Workflow: Find Fastest Cloudflare IPs

1.  **Install the tool**:
    
2.  **Run with default settings**:
    
3.  **View results in the terminal**:
    
    +   IPs will be sorted by latency
    +   Download speed will be displayed for each IP
    +   The best-performing IPs will be at the top

### Advanced Workflow: Customized Testing

1.  **Test with specific criteria**:
    
    This command:
    
    +   Uses TCP ping testing with port 443
    +   Sends 10 pings per IP
    +   Filters results to only show IPs with latency between 30ms and 200ms
    +   Shows only the top 5 results
2.  **Export results to CSV**:
    
3.  **Analyze the results** in the CSV file using a spreadsheet application
    

## Troubleshooting

### Permission Issues

On Unix-based systems (Linux/macOS), if you encounter permission issues:

### Network Connectivity Issues

+   Ensure you have internet connectivity
+   Check if firewall rules allow outbound connections on necessary ports
+   For ICMP tests, ensure you have permission to send ICMP packets (may require root/administrator privileges)

### Command-Line Argument Errors

If you encounter errors related to command-line arguments:

1.  Check the syntax and spelling of your options
2.  Ensure numerical values are within expected ranges
3.  Try running with fewer options to isolate the issue

## System Requirements

CloudflareST-Rust has minimal system requirements:

+   **Operating System**: Windows, macOS, or Linux
+   **Architecture**: x86\_64 (Intel/AMD) or ARM64
+   **Memory**: Minimal (less than 50MB)
+   **Disk Space**: Less than 10MB
+   **Network**: Internet connection to Cloudflare servers

No additional dependencies are required when using the pre-compiled binaries.
