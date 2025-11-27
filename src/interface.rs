use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

// 存储Windows系统下的接口索引
#[cfg(target_os = "windows")]
pub static mut INTERFACE_INDEX: Option<u32> = None;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;

#[cfg(target_os = "windows")]
use {
    std::os::windows::io::AsRawSocket,
    network_interface::{NetworkInterface, NetworkInterfaceConfig},
    windows_sys::Win32::Networking::WinSock::{
        setsockopt, IPPROTO_IP, IPPROTO_IPV6, IP_UNICAST_IF, IPV6_UNICAST_IF, SOCKET_ERROR,
    },
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
        ParsedInterface::Name(name) => {
            // 验证接口名是否有效
            let is_valid = is_valid_interface_name(&name);
            
            // 在Windows系统上，如果接口名有效，则将其转换为接口索引并存储到全局变量中
            #[cfg(target_os = "windows")]
            if is_valid && let Some(index) = get_interface_index(&name) {
                unsafe { INTERFACE_INDEX = Some(index) }
            }
            
            InterfaceParamResult { 
                interface_ips: None, 
                is_valid_interface: is_valid, 
            }
        }, 
    } 
}

/// 根据目标IP地址绑定源IP到socket
fn bind_source_ip_to_socket(sock: &TcpSocket, addr: &SocketAddr, ips: &InterfaceIps) -> Option<()> {
    #[cfg(target_os = "linux")]
    {
        let raw_fd = sock.as_raw_fd();
        let on: libc::c_int = 1;
        unsafe {
            libc::setsockopt(
                raw_fd,
                libc::SOL_IP,
                libc::IP_BIND_ADDRESS_NO_PORT,
                &on as *const _ as *const libc::c_void,
                std::mem::size_of_val(&on) as libc::socklen_t,
            );
        }
    }
    
    let ip = match addr.ip() { 
        IpAddr::V4(_) => ips.ipv4?, 
        IpAddr::V6(_) => ips.ipv6?, 
    }; 
    let port = ips.port.unwrap_or(0); 
    sock.bind(SocketAddr::new(ip, port)).ok() 
}

/// 根据IP地址类型创建对应的TCP Socket
fn create_tcp_socket_for_ip(addr: &IpAddr) -> Option<TcpSocket> {
    let sock = match addr {
        IpAddr::V4(_) => TcpSocket::new_v4().ok(),
        IpAddr::V6(_) => TcpSocket::new_v6().ok(),
    }?;
    
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

/// 绑定 TCP Socket
pub async fn bind_socket_to_interface(
    addr: SocketAddr,
    #[cfg(any(target_os = "linux", target_os = "macos"))] interface: Option<&str>,
    #[cfg(target_os = "windows")] _interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<TcpSocket> {
    // 创建基础socket
    let sock = create_tcp_socket_for_ip(&addr.ip())?;

    if let Some(ips) = interface_ips {
        // 如果提供了IP地址，则绑定IP地址
        bind_source_ip_to_socket(&sock, &addr, ips)?;
        return Some(sock);
    }

    // 使用全局变量中的接口索引
    #[cfg(target_os = "windows")]
    unsafe {
        if let Some(idx) = INTERFACE_INDEX {
            // 尝试绑定到接口索引
            if !bind_to_interface_index(&sock, idx, addr.is_ipv6()) {
                return None;
            }
        }
    }

    // 使用接口名
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // 如果提供了接口名，尝试绑定
        if let Some(name) = interface {
            bind_to_interface(&sock, name).ok()?;
        }
    }

    Some(sock)
}