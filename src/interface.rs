use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;
#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawSocket;

#[cfg(target_os = "windows")]
use network_interface::{NetworkInterface, NetworkInterfaceConfig};

// Windows 常量
#[cfg(target_os = "windows")]
const IPPROTO_IP: i32 = 0;
#[cfg(target_os = "windows")]
const IPPROTO_IPV6: i32 = 41;
#[cfg(target_os = "windows")]
const IP_UNICAST_IF: i32 = 31;
#[cfg(target_os = "windows")]
const IPV6_UNICAST_IF: i32 = 31;

/// 接口 IP 信息
#[derive(Clone)]
pub struct InterfaceIps {
    pub ipv4: Option<IpAddr>,
    pub ipv6: Option<IpAddr>,
    pub port: Option<u16>,
}

/// 接口解析结果
pub struct InterfaceParamResult {
    pub interface_ips: Option<InterfaceIps>,
    pub is_valid_interface: bool,
}

/// 解析接口参数类型
#[derive(Clone)]
pub enum ParsedInterface {
    SocketAddr(SocketAddr),
    Ip(IpAddr),
    Name(String),
}

/// 从 IP 和 port 构建 InterfaceIps 
fn interface_ips_from_ip(ip: IpAddr, port: Option<u16>) -> InterfaceIps { 
    match ip { 
        IpAddr::V4(ipv4) => InterfaceIps { ipv4: Some(ipv4.into()), ipv6: None, port }, 
        IpAddr::V6(ipv6) => InterfaceIps { ipv4: None, ipv6: Some(ipv6.into()), port }, 
    } 
} 

/// 解析接口参数（支持 IP、SocketAddr、接口名）
pub fn process_interface_param(interface: &str) -> InterfaceParamResult { 
    let parsed = if let Ok(socket_addr) = interface.parse::<SocketAddr>() { 
        ParsedInterface::SocketAddr(socket_addr) 
    } else if let Ok(ip) = interface.parse::<IpAddr>() { 
        ParsedInterface::Ip(ip) 
    } else { 
        ParsedInterface::Name(interface.to_string()) 
    }; 
 
    match parsed { 
        ParsedInterface::SocketAddr(addr) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(addr.ip(), Some(addr.port()))), 
            is_valid_interface: true, 
        }, 
        ParsedInterface::Ip(ip) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(ip, None)), 
            is_valid_interface: true, 
        }, 
        ParsedInterface::Name(name) => { 
            let valid = { 
                #[cfg(target_os = "windows")] 
                { get_interface_index(&name).is_some() } 
 
                #[cfg(any(target_os = "linux", target_os = "macos"))] 
                { 
                    std::ffi::CString::new(name.as_str()) 
                        .map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 }) 
                } 
            }; 
            InterfaceParamResult { interface_ips: None, is_valid_interface: valid } 
        } 
    } 
}

/// 根据目标IP地址绑定源IP到socket
fn bind_source_ip_to_socket(sock: &TcpSocket, addr: &SocketAddr, ips: &InterfaceIps) -> Option<()> {
    let source_ip = match addr.ip() {
        IpAddr::V4(_) => ips.ipv4,
        IpAddr::V6(_) => ips.ipv6,
    };

    // 只有源 IP 与目标 IP 协议族匹配才绑定，否则失败
    if let Some(ip) = source_ip {
        sock.bind(SocketAddr::new(ip, ips.port.unwrap_or(0))).ok()
    } else {
        // 明确拒绝不匹配的组合
        None
    }
}

/// 根据IP地址类型创建对应的TCP Socket
fn create_tcp_socket_for_ip(addr: &IpAddr) -> Option<TcpSocket> {
    match addr {
        IpAddr::V4(_) => TcpSocket::new_v4().ok(),
        IpAddr::V6(_) => TcpSocket::new_v6().ok(),
    }
}

/// 创建并绑定 TCP Socket
fn create_and_bind_tcp_socket(
    addr: &SocketAddr,
    ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    let sock = create_tcp_socket_for_ip(&addr.ip())?;

    if let Some(ips) = ips {
        bind_source_ip_to_socket(&sock, addr, ips)?;
    }

    Some(sock)
}

//
// 平台专用接口绑定函数
//

/// Linux/macOS: 按接口名绑定
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bind_to_interface(sock: &TcpSocket, name: &str) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let raw_fd = sock.as_raw_fd();
        let c_name = std::ffi::CString::new(name)?;
        let ret = unsafe {
            libc::setsockopt(
                raw_fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                c_name.as_ptr() as *const libc::c_void,
                name.len() as libc::socklen_t,
            )
        };
        if ret == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }
    
    #[cfg(target_os = "macos")]
    {
        let raw_fd = sock.as_raw_fd();
        let cname = std::ffi::CString::new(name)?;
        let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
        if idx == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let apply = |level, opt| unsafe {
            libc::setsockopt(
                raw_fd,
                level,
                opt,
                &idx as *const _ as *const libc::c_void,
                std::mem::size_of_val(&idx) as libc::socklen_t,
            )
        };

        if apply(libc::IPPROTO_IP, libc::IP_BOUND_IF) == 0
            || apply(libc::IPPROTO_IPV6, libc::IPV6_BOUND_IF) == 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
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

/// 平台特定的接口绑定
struct InterfaceBinder;

impl InterfaceBinder {
    /// 绑定 socket 到指定接口
    fn bind_to_interface(sock: &TcpSocket, name: &str, addr: &SocketAddr) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            bind_to_interface(sock, name).is_ok()
        }
        
        #[cfg(target_os = "windows")]
        {
            Self::bind_to_interface_windows(sock, name, addr.is_ipv6())
        }
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

//
// 主函数：绑定 socket
//

/// 核心函数：绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    // Windows平台保持原有逻辑
    #[cfg(target_os = "windows")]
    {
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
    
    #[cfg(not(target_os = "windows"))]
    {
        let sock = create_tcp_socket_for_ip(&addr.ip())?;

        // 接口绑定
        if let Some(name) = interface {
            if bind_to_interface(&sock, name).is_err() {
                return None;
            }
        }

        // 源IP绑定
        if let Some(ips) = interface_ips {
            bind_source_ip_to_socket(&sock, &addr, ips)?;
        }

        Some(sock)
    }
}
