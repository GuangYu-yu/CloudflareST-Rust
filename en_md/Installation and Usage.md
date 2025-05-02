## Installation and Usage

Related Source Files

+   [binaries/linux_amd64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/linux_amd64/CloudflareST-Rust)
+   [binaries/linux_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/linux_arm64/CloudflareST-Rust)
+   [binaries/macos_arm64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_arm64/CloudflareST-Rust)
+   [binaries/macos_x86_64/CloudflareST-Rust](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/macos_x86_64/CloudflareST-Rust)
+   [binaries/windows_arm64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_arm64/CloudflareST-Rust.exe)
+   [binaries/windows_x86_64/CloudflareST-Rust.exe](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/binaries/windows_x86_64/CloudflareST-Rust.exe)
+   [src/args.rs](https://github.com/GuangYu-yu/CloudflareST-Rust/blob/57de4236/src/args.rs)

This page provides comprehensive installation and usage instructions for CloudflareST-Rust, a network performance testing tool designed to evaluate and optimize connections with Cloudflare infrastructure. For project overview and features, please refer to [Overview](https://deepwiki.com/GuangYu-yu/CloudflareST-Rust/1-overview).

## Installation Options

CloudflareST-Rust provides pre-compiled binaries for multiple platforms, making the installation process straightforward for most users. You can also build the application from source if needed.

### Pre-compiled Binaries

#### Diagram: CloudflareST-Rust Binary Architectures

Source: `binaries/macos_arm64/CloudflareST-Rust`, `binaries/macos_x86_64/CloudflareST-Rust`, `binaries/windows_x86_64/CloudflareST-Rust.exe`, `binaries/windows_arm64/CloudflareST-Rust.exe`, `binaries/linux_amd64/CloudflareST-Rust`, `binaries/linux_arm64/CloudflareST-Rust`

#### Platform-Specific Installation

1.  **Windows**:
    
    +   Download the binary file (`CloudflareST-Rust.exe`) matching your architecture (x86_64 or ARM64)
    +   No installation required - can run the executable directly
    +   Optional: Add the executable to your PATH environment variable for easier access
2.  **macOS**:
    
    +   Download the binary file matching your architecture (Intel or Apple Silicon)
    +   Open Terminal and navigate to the download location
    +   Make the binary executable: `chmod +x CloudflareST-Rust`
    +   Optional: Move to a location in your PATH: `sudo mv CloudflareST-Rust /usr/local/bin/`
3.  **Linux**:
    
    +   Download the binary file matching your architecture (AMD64 or ARM64)
    +   Make the binary executable: `chmod +x CloudflareST-Rust`
    +   Optional: Move to a location in your PATH: `sudo mv CloudflareST-Rust /usr/local/bin/`

### Building from Source

To build from source, you will need:

+   Rust toolchain (rustc, cargo)
+   Git

1.  Clone the repository:

```
git clone https://github.com/GuangYu-yu/CloudflareST-Rust.git
cd CloudflareST-Rust
```
    
2.  Build the project:

```
cargo build --release
```
  
3.  The compiled binary will be in the `target/release/` directory
    

## Troubleshooting

### Permission Issues

On Unix-based systems (Linux/macOS), if you encounter permission issues:

### Network Connection Issues

+   Ensure you have internet connectivity
+   Check firewall rules to allow outbound connections on necessary ports
+   For ICMP testing, ensure you have permission to send ICMP packets (may require root/admin privileges)

### Command Line Argument Errors

If encountering errors related to command line arguments:

1.  Check option syntax and spelling
2.  Ensure numerical values are within expected ranges
3.  Try running with fewer options to isolate the issue

## System Requirements

CloudflareST-Rust has minimal system requirements:

+   **Operating System**: Windows, macOS or Linux
+   **Architecture**: x86_64 (Intel/AMD) or ARM64
+   **Memory**: Minimal
+   **Disk Space**: Less than 10MB

No additional dependencies are required when using pre-compiled binaries.
