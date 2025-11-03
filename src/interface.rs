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

/// Interface 参数处理结果
pub struct InterfaceParamResult {
    pub interface_ips: Option<InterfaceIps>,
    pub is_valid_interface: bool,
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
    let is_valid = {
        #[cfg(target_os = "windows")]
        {
            get_interface_index(interface).is_some()
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let c_name = std::ffi::CString::new(interface).ok();
            c_name.map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
        }
    };

    InterfaceParamResult {
        interface_ips: None,
        is_valid_interface: is_valid,
    }
}

/// 创建 TCP Socket 并可绑定 IP
fn create_and_bind_tcp_socket(addr: &SocketAddr, ips: Option<&InterfaceIps>) -> Option<TcpSocket> {
    let sock = match addr.ip() {
        IpAddr::V4(_) => TcpSocket::new_v4().ok()?,
        IpAddr::V6(_) => TcpSocket::new_v6().ok()?,
    };
    if let Some(ips) = ips {
        let source_ip = if addr.is_ipv4() { ips.ipv4 } else { ips.ipv6 };
        if let Some(ip) = source_ip {
            sock.bind(SocketAddr::new(ip, 0)).ok()?;
        }
    }
    Some(sock)
}

/// Linux: 按接口名绑定
#[cfg(target_os = "linux")]
fn bind_to_interface_name(sock: &Socket, iface_name: &str) -> bool {
    let c_name = match std::ffi::CString::new(iface_name) {
        Ok(name) => name,
        Err(_) => return false,
    };

    unsafe {
        return setsockopt(
            sock.as_raw_fd(),
            SOL_SOCKET,
            SO_BINDTODEVICE,
            c_name.as_ptr() as *const c_void,
            c_name.as_bytes_with_nul().len() as socklen_t,
        ) == 0;
    }
}

/// macOS: 按接口名绑定
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
    unsafe {
        return setsockopt(
            sock.as_raw_fd(),
            if is_ipv6 { IPPROTO_IPV6 } else { IPPROTO_IP },
            if is_ipv6 { IPV6_BOUND_IF } else { IP_BOUND_IF },
            &index as *const _ as *const c_void,
            std::mem::size_of_val(&index) as socklen_t,
        ) == 0;
    }
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
    if let Some(sock) = create_and_bind_tcp_socket(&addr, interface_ips) {
        return Some(sock);
    }

    // 接口名绑定
    if let Some(name) = interface {
        #[cfg(target_os = "linux")]
        {
            let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
            let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).ok()?;
            if !bind_to_interface_name(&sock, name) {
                return None;
            }
            // 设置为非阻塞模式，以便在异步环境中使用
            if let Err(_) = sock.set_nonblocking(true) {
                return None;
            }
            let std_stream: std::net::TcpStream = sock.into();
            return Some(TcpSocket::from(std_stream));
        }
        
        #[cfg(target_os = "macos")]
        {
            let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
            let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).ok()?;
            if !bind_to_interface_name(&sock, name, addr.is_ipv6()) {
                return None;
            }
            // 设置为非阻塞模式，以便在异步环境中使用
            if let Err(_) = sock.set_nonblocking(true) {
                return None;
            }
            let std_stream: std::net::TcpStream = sock.into();
            return Some(TcpSocket::from(std_stream));
        }

        #[cfg(target_os = "windows")]
        {
            let sock = create_and_bind_tcp_socket(&addr, None)?;
            let idx = get_interface_index(name)?;
            if !bind_to_interface_index(&sock, idx, addr.is_ipv6()) {
                return None;
            }
            return Some(sock);
        }
    }

    // 默认普通 socket
    create_and_bind_tcp_socket(&addr, None)
}