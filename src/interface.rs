use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use std::net::IpAddr;
use std::net::SocketAddr;
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use socket2::{Domain, Protocol, Socket, Type};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;

#[cfg(target_os = "linux")]
use libc::{SO_BINDTODEVICE, SOL_SOCKET, c_void, setsockopt, socklen_t};

#[cfg(target_os = "macos")]
use libc::{
    IP_BOUND_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_BOUND_IF, c_void, if_nametoindex, setsockopt,
    socklen_t,
};

/// 网络接口的 IP 地址信息
#[derive(Clone)]
pub struct InterfaceIps {
    pub ipv4: Option<IpAddr>,
    pub ipv6: Option<IpAddr>,
}

/// 接口参数处理结果
#[derive(Clone)]
pub struct InterfaceParamResult {
    pub interface_ips: Option<InterfaceIps>,
    pub is_valid_interface: bool,
}

/// 获取网络接口的 IPv4 和 IPv6 地址
pub fn get_interface_ip(interface_name: &str) -> Option<InterfaceIps> {
    // 获取所有网络接口
    let interfaces = NetworkInterface::show().ok()?;

    // 查找指定名称的接口
    for interface in interfaces {
        if interface.name == interface_name {
            let mut ipv4 = None;
            let mut ipv6 = None;

            // 收集接口的所有 IP 地址
            for addr in interface.addr {
                match addr.ip() {
                    IpAddr::V4(ipv4_addr) => {
                        // 过滤掉环回地址、链路本地地址和多播地址
                        if ipv4.is_none()
                            && !ipv4_addr.is_loopback()
                            && !ipv4_addr.is_link_local()
                            && !ipv4_addr.is_multicast()
                        {
                            ipv4 = Some(IpAddr::V4(ipv4_addr));
                        }
                    }
                    IpAddr::V6(ipv6_addr) => {
                        // 过滤掉环回地址、单播链路本地地址和多播地址
                        if ipv6.is_none()
                            && !ipv6_addr.is_loopback()
                            && !ipv6_addr.is_unicast_link_local()
                            && !ipv6_addr.is_multicast()
                        {
                            ipv6 = Some(IpAddr::V6(ipv6_addr));
                        }
                    }
                }
            }

            return Some(InterfaceIps { ipv4, ipv6 });
        }
    }

    None
}

/// 处理接口参数，根据参数类型返回相应的结果
pub fn process_interface_param(interface: &str) -> InterfaceParamResult {
    // 尝试解析为IP地址
    if let Ok(ip_addr) = interface.parse::<std::net::IpAddr>() {
        // 如果是IP地址，创建 InterfaceIps 结构体
        let interface_ips = match ip_addr {
            IpAddr::V4(ipv4) => Some(InterfaceIps {
                ipv4: Some(IpAddr::V4(ipv4)),
                ipv6: None,
            }),
            IpAddr::V6(ipv6) => Some(InterfaceIps {
                ipv4: None,
                ipv6: Some(IpAddr::V6(ipv6)),
            }),
        };

        InterfaceParamResult {
            interface_ips,
            is_valid_interface: true,
        }
    } else {
        // 如果是接口名
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            // Linux 和 macOS 系统：检查接口是否存在，但保留接口名，不获取 IP 地址
            let is_valid = NetworkInterface::show().map_or(false, |ints| {
                ints.iter().any(|iface| iface.name == interface)
            });

            InterfaceParamResult {
                interface_ips: None,
                is_valid_interface: is_valid,
            }
        }

        #[cfg(target_os = "windows")]
        {
            // Windows 系统：尝试获取接口的 IP 地址
            let interface_ips = get_interface_ip(interface);
            let is_valid = interface_ips.is_some();

            InterfaceParamResult {
                interface_ips,
                is_valid_interface: is_valid,
            }
        }
    }
}

/// 创建 TcpSocket 的统一逻辑
fn create_tcp_socket(addr: &SocketAddr) -> Option<TcpSocket> {
    match addr.ip() {
        IpAddr::V4(_) => TcpSocket::new_v4().ok(),
        IpAddr::V6(_) => TcpSocket::new_v6().ok(),
    }
}

/// InterfaceIps 绑定优先 IP 的统一逻辑
fn bind_to_ip(sock: &TcpSocket, addr: &SocketAddr, ips: &InterfaceIps) -> Option<()> {
    let source_ip = match addr.ip() {
        IpAddr::V4(_) => ips.ipv4,
        IpAddr::V6(_) => ips.ipv6,
    };
    if let Some(ip) = source_ip {
        sock.bind(SocketAddr::new(ip, 0)).ok()?;
    }
    Some(())
}

/// 通用的接口绑定函数，用于创建绑定到指定接口的TCP socket
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    // 优先处理 interface_ips（绑定到具体 IP 地址）
    if let Some(ips) = interface_ips {
        let sock = create_tcp_socket(&addr)?;
        bind_to_ip(&sock, &addr, ips)?;
        return Some(sock);
    }

    // 处理 interface（绑定到接口名）
    if let Some(intf) = interface {
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let sock2 = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).ok()?;
        let c_name = std::ffi::CString::new(intf).ok()?;

        #[cfg(target_os = "linux")]
        {
            let result = unsafe {
                setsockopt(
                    sock2.as_raw_fd(),
                    SOL_SOCKET,
                    SO_BINDTODEVICE,
                    c_name.as_ptr() as *const c_void,
                    c_name.as_bytes_with_nul().len() as socklen_t,
                )
            };

            if result != 0 {
                return None; // 绑定失败
            }
        }

        #[cfg(target_os = "macos")]
        {
            let interface_index = unsafe { if_nametoindex(c_name.as_ptr()) };

            if interface_index == 0 {
                return None; // 接口不存在
            }

            // 对于 IPv4
            let result_v4 = unsafe {
                setsockopt(
                    sock2.as_raw_fd(),
                    IPPROTO_IP,
                    IP_BOUND_IF,
                    &interface_index as *const _ as *const c_void,
                    std::mem::size_of_val(&interface_index) as socklen_t,
                )
            };

            // 对于 IPv6
            let result_v6 = unsafe {
                setsockopt(
                    sock2.as_raw_fd(),
                    IPPROTO_IPV6,
                    IPV6_BOUND_IF,
                    &interface_index as *const _ as *const c_void,
                    std::mem::size_of_val(&interface_index) as socklen_t,
                )
            };

            if result_v4 != 0 && result_v6 != 0 {
                return None; // 绑定失败
            }
        }

        let std_stream: std::net::TcpStream = sock2.into();
        return Some(TcpSocket::from_std_stream(std_stream));
    }

    // 都没有提供，普通创建
    create_tcp_socket(&addr)
}

/// Windows系统的接口绑定函数（占位符）
#[cfg(target_os = "windows")]
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    let _ = interface;

    // 统一使用 create_tcp_socket 和 bind_to_ip，无论是否有 interface_ips
    let sock = create_tcp_socket(&addr)?;
    if let Some(ips) = interface_ips {
        // Windows 无法直接绑定接口名，但可以通过绑定解析后的 IP 地址
        bind_to_ip(&sock, &addr, ips)?;
    }
    Some(sock)
}