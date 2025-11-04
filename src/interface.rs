use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;

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
    pub port: Option<u16>,
}

/// Interface 参数处理结果
pub struct InterfaceParamResult {
    pub interface_ips: Option<InterfaceIps>,
    pub is_valid_interface: bool,
}

/// 解析后的接口参数
#[derive(Clone)]
pub enum ParsedInterface {
    SocketAddr(SocketAddr),
    Ip(IpAddr),
    Name(String),
}

/// 解析接口参数
pub fn process_interface_param(interface: &str) -> InterfaceParamResult {
    // 统一解析函数
    let parsed = if let Ok(socket_addr) = interface.parse::<SocketAddr>() {
        ParsedInterface::SocketAddr(socket_addr)
    } else if let Ok(ip) = interface.parse::<IpAddr>() {
        ParsedInterface::Ip(ip)
    } else {
        ParsedInterface::Name(interface.to_string())
    };

    match parsed {
        ParsedInterface::SocketAddr(addr) => {
            let ips = match addr.ip() {
                IpAddr::V4(ipv4) => InterfaceIps { ipv4: Some(ipv4.into()), ipv6: None, port: Some(addr.port()) },
                IpAddr::V6(ipv6) => InterfaceIps { ipv4: None, ipv6: Some(ipv6.into()), port: Some(addr.port()) },
            };
            InterfaceParamResult { interface_ips: Some(ips), is_valid_interface: true }
        }
        ParsedInterface::Ip(ip) => {
            let ips = match ip {
                IpAddr::V4(ipv4) => InterfaceIps { ipv4: Some(ipv4.into()), ipv6: None, port: None },
                IpAddr::V6(ipv6) => InterfaceIps { ipv4: None, ipv6: Some(ipv6.into()), port: None },
            };
            InterfaceParamResult { interface_ips: Some(ips), is_valid_interface: true }
        }
        ParsedInterface::Name(name) => {
            let is_valid = {
                #[cfg(target_os = "windows")]
                { get_interface_index(&name).is_some() }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    let c_name = std::ffi::CString::new(name.as_str()).ok();
                    c_name.map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
                }
            };
            InterfaceParamResult { interface_ips: None, is_valid_interface: is_valid }
        }
    }
}

/// 创建 TCP Socket 并可绑定 IP
fn create_and_bind_tcp_socket(addr: &SocketAddr, ips: Option<&InterfaceIps>) -> Option<TcpSocket> {
    let sock = match addr.ip() {
        IpAddr::V4(_) => TcpSocket::new_v4().ok()?,
        IpAddr::V6(_) => TcpSocket::new_v6().ok()?,
    };

    if let Some(ips) = ips {
        let source_ip = match addr.ip() {
            IpAddr::V4(_) => ips.ipv4,
            IpAddr::V6(_) => ips.ipv6,
        };

        // 只有源 IP 与目标 IP 协议族匹配才绑定，否则失败
        if let Some(ip) = source_ip {
            sock.bind(SocketAddr::new(ip, ips.port.unwrap_or(0))).ok()?;
        } else {
            // 明确拒绝不匹配的组合
            return None;
        }
    }

    Some(sock)
}

/// Linux: 按接口名绑定
#[cfg(target_os = "linux")]
fn bind_to_interface_name_linux(sock: &TcpSocket, name: &str) -> bool {
    let raw_fd = sock.as_raw_fd();
    let c_name = match std::ffi::CString::new(name) {
        Ok(c_name) => c_name,
        Err(_) => return false,
    };
    let result = unsafe {
        libc::setsockopt(
            raw_fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            c_name.as_ptr() as *const libc::c_void,
            name.len() as libc::socklen_t,
        )
    };
    result == 0
}

/// macOS: 按接口名绑定
#[cfg(target_os = "macos")]
fn bind_to_interface_name_macos(sock: &TcpSocket, name: &str) -> bool {
    let raw_fd = sock.as_raw_fd();
    let interface_name_cstr = match std::ffi::CString::new(name) {
        Ok(c_name) => c_name,
        Err(_) => return false,
    };
    let interface_index =
        unsafe { libc::if_nametoindex(interface_name_cstr.as_ptr() as *const libc::c_char) };
    if interface_index == 0 {
        return false;
    }
    
    let apply = |level, opt| unsafe {
        libc::setsockopt(
            raw_fd,
            level,
            opt,
            &interface_index as *const _ as *const libc::c_void,
            std::mem::size_of_val(&interface_index) as libc::socklen_t,
        )
    };
    
    // 尝试绑定IPv4和IPv6，只要有一个成功即可
    apply(libc::IPPROTO_IP, libc::IP_BOUND_IF) == 0 || 
    apply(libc::IPPROTO_IPV6, libc::IPV6_BOUND_IF) == 0
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

/// 平台特定的接口绑定
struct InterfaceBinder;

impl InterfaceBinder {
    /// 绑定 socket 到指定接口
    fn bind_to_interface(sock: &TcpSocket, name: &str, addr: &SocketAddr) -> bool {
        #[cfg(target_os = "linux")]
        {
            Self::bind_to_interface_linux(sock, name)
        }
        
        #[cfg(target_os = "macos")]
        {
            Self::bind_to_interface_macos(sock, name)
        }
        
        #[cfg(target_os = "windows")]
        {
            Self::bind_to_interface_windows(sock, name, addr.is_ipv6())
        }
    }
    
    #[cfg(target_os = "linux")]
    fn bind_to_interface_linux(sock: &TcpSocket, name: &str) -> bool {
        bind_to_interface_name_linux(sock, name)
    }
    
    #[cfg(target_os = "macos")]
    fn bind_to_interface_macos(sock: &TcpSocket, name: &str) -> bool {
        bind_to_interface_name_macos(sock, name)
    }
    
    #[cfg(target_os = "windows")]
    fn bind_to_interface_windows(sock: &TcpSocket, name: &str, is_ipv6: bool) -> bool {
        if let Some(idx) = get_interface_index(name) {
            bind_to_interface_index(sock, idx, is_ipv6)
        } else {
            false
        }
    }
}

/// 核心函数：绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    // 1. 优先尝试 IP 绑定
    if let Some(sock) = create_and_bind_tcp_socket(&addr, interface_ips) {
        return Some(sock);
    }
    
    // 2. 尝试接口名绑定
    if let Some(name) = interface {
        let sock = create_and_bind_tcp_socket(&addr, None)?;
        if InterfaceBinder::bind_to_interface(&sock, name, &addr) {
            return Some(sock);
        }
        // 接口绑定失败
        return None;
    }
    
    // 3. 只有在没有指定接口时才返回默认普通 socket
    create_and_bind_tcp_socket(&addr, None)
}
