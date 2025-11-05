use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;
#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawSocket;

#[cfg(any(target_os = "linux", target_os = "windows"))]
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

/// 从 IP 和 port 构建 InterfaceIps
fn interface_ips_from_ip(ip: IpAddr, port: Option<u16>) -> InterfaceIps {
    match ip {
        IpAddr::V4(ipv4) => InterfaceIps { ipv4: Some(ipv4.into()), ipv6: None, port },
        IpAddr::V6(ipv6) => InterfaceIps { ipv4: None, ipv6: Some(ipv6.into()), port },
    }
}

/// 解析接口参数（支持 IP、SocketAddr、接口名）
pub fn process_interface_param(interface: &str) -> InterfaceParamResult {
    if let Ok(addr) = interface.parse::<SocketAddr>() {
        return InterfaceParamResult {
            interface_ips: Some(interface_ips_from_ip(addr.ip(), Some(addr.port()))),
            is_valid_interface: true,
        };
    }

    if let Ok(ip) = interface.parse::<IpAddr>() {
        return InterfaceParamResult {
            interface_ips: Some(interface_ips_from_ip(ip, None)),
            is_valid_interface: true,
        };
    }

    let valid = {
        #[cfg(target_os = "windows")]
        { get_interface_index(interface).is_some() }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            std::ffi::CString::new(interface)
                .map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
        }
    };

    InterfaceParamResult { interface_ips: None, is_valid_interface: valid }
}

/// 根据目标IP地址和协议族选取对应源IP
fn select_ip_by_family(addr: &SocketAddr, ips: &InterfaceIps) -> Option<IpAddr> {
    match addr.ip() {
        IpAddr::V4(_) => ips.ipv4,
        IpAddr::V6(_) => ips.ipv6,
    }
}

/// 根据目标IP地址绑定源IP到socket
fn bind_source_ip_to_socket(sock: &TcpSocket, addr: &SocketAddr, ips: &InterfaceIps) -> Option<()> {
    if let Some(ip) = select_ip_by_family(addr, ips) {
        sock.bind(SocketAddr::new(ip, ips.port.unwrap_or(0))).ok()
    } else {
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

/// 创建并绑定 TCP Socket（仅源IP）
fn create_and_bind_tcp_socket(addr: &SocketAddr, ips: Option<&InterfaceIps>) -> Option<TcpSocket> {
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

/// Linux: 根据源IP查找对应的接口名
#[cfg(target_os = "linux")]
fn get_interface_name_by_ip(ip: &IpAddr) -> Option<String> {
    let interfaces = NetworkInterface::show().ok()?;
    for iface in interfaces {
        for addr in &iface.addr {
            if addr.ip() == *ip {
                return Some(iface.name.clone());
            }
        }
    }
    None
}

/// 平台特定的接口绑定封装
struct InterfaceBinder;

impl InterfaceBinder {
    fn bind(sock: &TcpSocket, name: &str, addr: &SocketAddr) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        { bind_to_interface(sock, name).is_ok() }

        #[cfg(target_os = "windows")]
        { Self::bind_to_interface_windows(sock, name, addr.is_ipv6()) }
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

#[cfg(target_os = "windows")]
async fn bind_socket_windows(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    if let Some(sock) = create_and_bind_tcp_socket(&addr, interface_ips) {
        return Some(sock);
    }

    if let Some(name) = interface {
        let sock = create_and_bind_tcp_socket(&addr, None)?;
        if InterfaceBinder::bind(&sock, name, &addr) {
            return Some(sock);
        }
        return None;
    }

    create_and_bind_tcp_socket(&addr, None)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn bind_socket_unix(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    let sock = create_tcp_socket_for_ip(&addr.ip())?;
    let mut iface_to_bind = interface.map(|s| s.to_string());

    #[cfg(target_os = "linux")]
    if iface_to_bind.is_none() {
        if let Some(ips) = interface_ips {
            if let Some(ip) = select_ip_by_family(&addr, ips) {
                iface_to_bind = get_interface_name_by_ip(&ip);
            }
        }
    }

    if let Some(name) = iface_to_bind {
        if !InterfaceBinder::bind(&sock, &name, &addr) {
            return None;
        }
    }

    if let Some(ips) = interface_ips {
        bind_source_ip_to_socket(&sock, &addr, ips)?;
    }

    Some(sock)
}

/// 核心函数：绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    #[cfg(target_os = "windows")]
    { bind_socket_windows(addr, interface, interface_ips).await }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    { bind_socket_unix(addr, interface, interface_ips).await }
}
