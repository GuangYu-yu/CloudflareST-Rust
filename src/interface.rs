use std::net::{IpAddr, SocketAddr};
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

#[cfg(target_os = "windows")]
use network_interface::{NetworkInterface, NetworkInterfaceConfig};
#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawSocket;

// Windows 常量
#[cfg(target_os = "windows")]
const IPPROTO_IP: i32 = 0;
#[cfg(target_os = "windows")]
const IPPROTO_IPV6: i32 = 41;
#[cfg(target_os = "windows")]
const IP_UNICAST_IF: i32 = 31;
#[cfg(target_os = "windows")]
const IPV6_UNICAST_IF: i32 = 31;

/// Interface IP 信息
#[derive(Clone)]
pub struct InterfaceIps {
    pub ipv4: Option<IpAddr>,
    pub ipv6: Option<IpAddr>,
}

/// 创建 TCP Socket
fn create_tcp_socket(addr: &SocketAddr) -> Option<TcpSocket> {
    match addr.ip() {
        IpAddr::V4(_) => TcpSocket::new_v4().ok(),
        IpAddr::V6(_) => TcpSocket::new_v6().ok(),
    }
}

/// 优先绑定 IP
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

/// Interface 参数处理结果
pub struct InterfaceParamResult {
    pub interface_ips: Option<InterfaceIps>,
    pub is_valid_interface: bool,
}

/// 获取网络接口的 IPv4 和 IPv6 地址
#[cfg(target_os = "windows")]
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

/// 解析接口参数
pub fn process_interface_param(interface: &str) -> InterfaceParamResult {
    // 尝试解析为 IP
    if let Ok(ip) = interface.parse::<IpAddr>() {
        let ips = match ip {
            IpAddr::V4(ipv4) => InterfaceIps {
                ipv4: Some(IpAddr::V4(ipv4)),
                ipv6: None,
            },
            IpAddr::V6(ipv6) => InterfaceIps {
                ipv4: None,
                ipv6: Some(IpAddr::V6(ipv6)),
            },
        };
        return InterfaceParamResult {
            interface_ips: Some(ips),
            is_valid_interface: true,
        };
    }

    // 否则当作接口名
    #[cfg(target_os = "windows")]
    {
        if let Some(ips) = crate::interface::get_interface_ip(interface) {
            return InterfaceParamResult {
                interface_ips: Some(ips),
                is_valid_interface: true,
            };
        } else {
            return InterfaceParamResult {
                interface_ips: None,
                is_valid_interface: false,
            };
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let is_valid = {
            let c_name = std::ffi::CString::new(interface).ok();
            c_name.map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
        };
        InterfaceParamResult {
            interface_ips: None,
            is_valid_interface: is_valid,
        }
    }
}

/// Linux: SO_BINDTODEVICE
#[cfg(target_os = "linux")]
fn bind_to_interface_name(sock: &Socket, iface_name: &str) -> bool {
    let c_name = match std::ffi::CString::new(iface_name) {
        Ok(name) => name,
        Err(_) => return false,
    };
    let res = unsafe {
        setsockopt(
            sock.as_raw_fd(),
            SOL_SOCKET,
            SO_BINDTODEVICE,
            c_name.as_ptr() as *const c_void,
            c_name.as_bytes_with_nul().len() as socklen_t,
        )
    };
    res == 0
}

/// macOS: IP_BOUND_IF / IPV6_BOUND_IF
#[cfg(target_os = "macos")]
fn bind_to_interface_name(sock: &Socket, iface_name: &str, is_ipv6: bool) -> bool {
    let c_name = match std::ffi::CString::new(iface_name) {
        Ok(name) => name,
        Err(_) => return false,
    };
    let index = unsafe { if_nametoindex(c_name.as_ptr()) };
    if index == 0 {
        return false;
    }
    let res = unsafe {
        if is_ipv6 {
            setsockopt(
                sock.as_raw_fd(),
                IPPROTO_IPV6,
                IPV6_BOUND_IF,
                &index as *const _ as *const c_void,
                std::mem::size_of_val(&index) as socklen_t,
            )
        } else {
            setsockopt(
                sock.as_raw_fd(),
                IPPROTO_IP,
                IP_BOUND_IF,
                &index as *const _ as *const c_void,
                std::mem::size_of_val(&index) as socklen_t,
            )
        }
    };
    res == 0
}

/// Windows: 按接口索引绑定
#[cfg(target_os = "windows")]
fn bind_to_interface_index(sock: &TcpSocket, iface_idx: u32, is_ipv6: bool) -> bool {
    let raw = sock.as_raw_socket();
    let (level, option) = if is_ipv6 {
        (IPPROTO_IPV6, IPV6_UNICAST_IF)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF)
    };
    let res = unsafe {
        libc::setsockopt(
            raw as _,
            level,
            option,
            &iface_idx as *const _ as *const _,
            std::mem::size_of_val(&iface_idx) as i32,
        )
    };
    res == 0
}

/// Windows: 获取接口索引
#[cfg(target_os = "windows")]
pub fn get_interface_index(name: &str) -> Option<u32> {
    let interfaces = NetworkInterface::show().ok()?;
    for iface in interfaces {
        if iface.name == name {
            return Some(iface.index);
        }
    }
    None
}

/// 核心函数：绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    // 优先 IP
    if let Some(ips) = interface_ips {
        let sock = create_tcp_socket(&addr)?;
        bind_to_ip(&sock, &addr, ips)?;
        return Some(sock);
    }

    // 接口名绑定
    if let Some(name) = interface {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let domain = if addr.is_ipv4() {
                Domain::IPV4
            } else {
                Domain::IPV6
            };
            let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).ok()?;
            let success = {
                #[cfg(target_os = "linux")]
                {
                    bind_to_interface_name(&sock, name)
                }
                #[cfg(target_os = "macos")]
                {
                    bind_to_interface_name(&sock, name, addr.is_ipv6())
                }
            };
            if !success {
                return None;
            }
            let std_stream: std::net::TcpStream = sock.into();
            return Some(TcpSocket::from_std_stream(std_stream));
        }

        #[cfg(target_os = "windows")]
        {
            let sock = create_tcp_socket(&addr)?;
            let idx = get_interface_index(name)?;
            if !bind_to_interface_index(&sock, idx, addr.is_ipv6()) {
                return None;
            }
            return Some(sock);
        }
    }

    // 默认普通 socket
    create_tcp_socket(&addr)
}
