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
            let valid = {
                #[cfg(target_os = "windows")]
                { get_interface_index(&name).is_some() }

                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    let c_name = std::ffi::CString::new(name.as_str()).ok();
                    c_name.map_or(false, |c| unsafe { libc::if_nametoindex(c.as_ptr()) != 0 })
                }
            };
            InterfaceParamResult { interface_ips: None, is_valid_interface: valid }
        }
    }
}

//
// 平台专用接口绑定函数
//

#[cfg(target_os = "linux")]
fn bind_to_interface(sock: &TcpSocket, name: &str) -> std::io::Result<()> {
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
fn bind_to_interface(sock: &TcpSocket, name: &str) -> std::io::Result<()> {
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

#[cfg(target_os = "windows")]
fn bind_to_interface(sock: &TcpSocket, name: &str, is_ipv6: bool) -> std::io::Result<()> {
    let idx = get_interface_index(name).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "interface not found"))?;
    let raw = sock.as_raw_socket();
    let (level, opt) = if is_ipv6 { (IPPROTO_IPV6, IPV6_UNICAST_IF) } else { (IPPROTO_IP, IP_UNICAST_IF) };
    let ret = unsafe {
        libc::setsockopt(
            raw as _,
            level,
            opt,
            &idx as *const _ as *const _,
            std::mem::size_of_val(&idx) as i32,
        )
    };
    if ret == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(target_os = "windows")]
pub fn get_interface_index(name: &str) -> Option<u32> {
    let interfaces = NetworkInterface::show().ok()?;
    interfaces.into_iter().find(|i| i.name == name).map(|i| i.index)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn interface_for_ip(ip: IpAddr) -> Option<String> {
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

/// 核心函数：绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    let sock = match addr.ip() {
        IpAddr::V4(_) => TcpSocket::new_v4().ok()?,
        IpAddr::V6(_) => TcpSocket::new_v6().ok()?,
    };

    // 如果指定了接口名
    if let Some(name) = interface {
        let ok = {
            #[cfg(target_os = "windows")]
            { bind_to_interface(&sock, name, addr.is_ipv6()).is_ok() }
            #[cfg(not(target_os = "windows"))]
            { bind_to_interface(&sock, name).is_ok() }
        };
        if !ok { return None; }
    }

    // 如果只指定了源IP
    if interface.is_none() {
        if let Some(ips) = &interface_ips {
            let source_ip = match addr.ip() {
                IpAddr::V4(_) => ips.ipv4,
                IpAddr::V6(_) => ips.ipv6,
            }?;
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                if let Some(if_name) = interface_for_ip(source_ip) {
                    if bind_to_interface(&sock, &if_name).is_err() {
                        return None;
                    }
                }
            }
            sock.bind(SocketAddr::new(source_ip, ips.port.unwrap_or(0))).ok()?;
        }
    } else if let Some(ips) = &interface_ips {
        // 同时指定接口名和源IP，先接口再bind
        let source_ip = match addr.ip() {
            IpAddr::V4(_) => ips.ipv4,
            IpAddr::V6(_) => ips.ipv6,
        }?;
        sock.bind(SocketAddr::new(source_ip, ips.port.unwrap_or(0))).ok()?;
    }

    Some(sock)
}
