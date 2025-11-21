use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;
#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawSocket;

#[cfg(target_os = "windows")]
use network_interface::{NetworkInterface, NetworkInterfaceConfig};

// 导入 Windows FFI 库中需要的常量和函数
#[cfg(target_os = "windows")]
use windows_sys::Win32::Networking::WinSock::{
    setsockopt, IPPROTO_IP, IPPROTO_IPV6, IP_UNICAST_IF, IPV6_UNICAST_IF, SOCKET_ERROR,
};

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

/// 验证接口名是否有效
fn is_valid_interface_name(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    { get_interface_index(name).is_some() }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        std::ffi::CString::new(name)
            .map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
    }
}

/// 解析接口参数（支持 IP、SocketAddr、接口名）
pub fn process_interface_param(interface: &str) -> InterfaceParamResult { 
    let parsed = interface.parse::<SocketAddr>()
        .map(ParsedInterface::SocketAddr)
        .or_else(|_| interface.parse::<IpAddr>().map(ParsedInterface::Ip))
        .unwrap_or_else(|_| ParsedInterface::Name(interface.to_string()));
    
    match parsed { 
        ParsedInterface::SocketAddr(addr) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(addr.ip(), Some(addr.port()))), 
            is_valid_interface: true, 
        }, 
        ParsedInterface::Ip(ip) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(ip, None)), 
            is_valid_interface: true, 
        }, 
        ParsedInterface::Name(name) => InterfaceParamResult { 
            interface_ips: None, 
            is_valid_interface: is_valid_interface_name(&name), 
        }, 
    } 
}

/// 根据目标IP地址绑定源IP到socket
fn bind_source_ip_to_socket(sock: &TcpSocket, addr: &SocketAddr, ips: &InterfaceIps) -> Option<()> {
    let ip = match addr.ip() { 
        IpAddr::V4(_) => ips.ipv4?, 
        IpAddr::V6(_) => ips.ipv6?, 
    }; 
    let port = ips.port.unwrap_or(0); 
    sock.bind(SocketAddr::new(ip, port)).ok() 
}

/// 根据IP地址类型创建对应的TCP Socket
fn create_tcp_socket_for_ip(addr: &IpAddr) -> Option<TcpSocket> {
    match addr {
        IpAddr::V4(_) => TcpSocket::new_v4().ok(),
        IpAddr::V6(_) => TcpSocket::new_v6().ok(),
    }
}

#[cfg(target_os = "windows")]
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
        
        if ret == 0 { 
            Ok(()) 
        } else { 
            Err(std::io::Error::last_os_error())
        }
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
    
    let res = if is_ipv6 {
        let idx_bytes = iface_idx.to_ne_bytes();
        unsafe {
            setsockopt(
                raw as _,
                IPPROTO_IPV6,
                IPV6_UNICAST_IF,
                idx_bytes.as_ptr() as *const _,
                idx_bytes.len() as i32,
            )
        }
    } else {
        let idx_bytes = iface_idx.to_be_bytes();
        unsafe {
            setsockopt(
                raw as _,
                IPPROTO_IP,
                IP_UNICAST_IF,
                idx_bytes.as_ptr() as *const _,
                idx_bytes.len() as i32,
            )
        }
    };
    
    res != SOCKET_ERROR
}

/// Windows: 获取接口索引
#[cfg(target_os = "windows")]
pub fn get_interface_index(name: &str) -> Option<u32> {
    NetworkInterface::show().ok()?
        .into_iter()
        .find(|iface| iface.name == name)
        .map(|iface| iface.index)
}

/// Linux: IP 对应 接口名
#[cfg(target_os = "linux")]
pub fn find_interface_by_ip(ip: &IpAddr) -> Option<String> { 
    unsafe { 
        let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut(); 
        if libc::getifaddrs(&mut ifaddrs) != 0 { 
            return None; 
        } 
 
        let mut ptr = ifaddrs; 
        while !ptr.is_null() { 
            let ifa = &*ptr; 
            if !ifa.ifa_addr.is_null() { 
                let sa = &*ifa.ifa_addr; 
                match sa.sa_family as i32 { 
                    libc::AF_INET => { 
                        if let IpAddr::V4(ipv4) = ip { 
                            let sa_in: &libc::sockaddr_in = &*(ifa.ifa_addr as *const libc::sockaddr_in); 
                            if IpAddr::V4(std::net::Ipv4Addr::from(u32::from_be(sa_in.sin_addr.s_addr))) == ipv4 { 
                                let cstr = std::ffi::CStr::from_ptr(ifa.ifa_name); 
                                let name = cstr.to_string_lossy().to_string(); 
                                libc::freeifaddrs(ifaddrs); 
                                return Some(name); 
                            } 
                        } 
                    } 
                    libc::AF_INET6 => { 
                        if let IpAddr::V6(ipv6) = ip { 
                            let sa_in6: &libc::sockaddr_in6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6); 
                            let ip_bytes = sa_in6.sin6_addr.s6_addr; 
                            if IpAddr::V6(std::net::Ipv6Addr::from(ip_bytes)) == ipv6 { 
                                let cstr = std::ffi::CStr::from_ptr(ifa.ifa_name); 
                                let name = cstr.to_string_lossy().to_string(); 
                                libc::freeifaddrs(ifaddrs); 
                                return Some(name); 
                            } 
                        } 
                    } 
                    _ => {} 
                } 
            } 
            ptr = (*ptr).ifa_next; 
        } 
        libc::freeifaddrs(ifaddrs); 
        None 
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
            if let Some(idx) = get_interface_index(name) {
                let sock = create_and_bind_tcp_socket(&addr, None)?;
                if bind_to_interface_index(&sock, idx, addr.is_ipv6()) {
                    return Some(sock);
                }
            }
            // 接口绑定失败
            return None;
        }
        
        // 3. 只有在没有指定接口时才返回默认普通 socket
        create_and_bind_tcp_socket(&addr, None)
    }
    
    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "linux")]
    {
        let sock = create_tcp_socket_for_ip(&addr.ip())?;
        
        // 确定要绑定的接口名
        let source_ip = interface_ips.and_then(|ips| match addr.ip() {
            IpAddr::V4(_) => ips.ipv4,
            IpAddr::V6(_) => ips.ipv6,
        });
        
        let interface_name = interface.map(|s| s.to_string())
            .or_else(|| source_ip.and_then(|ip| find_interface_by_ip(&ip)));
        
        // 接口绑定
        if let Some(name) = &interface_name {
            if bind_to_interface(&sock, name).is_err() {
                return None;
            }
        }
        
        // 源IP绑定（在接口绑定后额外进行bind操作）
        if let Some(ips) = interface_ips {
            bind_source_ip_to_socket(&sock, &addr, ips)?;
        }

        Some(sock)
    }
}
