use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpSocket;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::AsRawFd;

#[cfg(target_os = "windows")]
use {
    std::os::windows::io::AsRawSocket,
    windows_sys::Win32::{
        Foundation::{
            ERROR_BUFFER_OVERFLOW,
            ERROR_SUCCESS,
        },
        Networking::WinSock::{
            setsockopt, IPPROTO_IP, IPPROTO_IPV6, IP_UNICAST_IF, IPV6_UNICAST_IF, SOCKET_ERROR,
            AF_UNSPEC,
        },
        NetworkManagement::IpHelper::{
            GetAdaptersAddresses,
            IP_ADAPTER_ADDRESSES_LH,
            GAA_FLAG_INCLUDE_PREFIX,
        },
    },
};

/// 接口 IP 信息
#[derive(Clone)]
pub(crate) struct InterfaceIps {
    pub(crate) ipv4: Option<IpAddr>,
    pub(crate) ipv6: Option<IpAddr>,
    pub(crate) port: Option<u16>,
}

/// 接口解析结果
#[derive(Clone, Default)]
pub(crate) struct InterfaceParamResult {
    pub(crate) interface_ips: Option<InterfaceIps>,
    pub(crate) is_valid_interface: bool,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) name: Option<String>,
    #[cfg(target_os = "windows")]
    pub(crate) interface_index: Option<u32>,
}

/// 解析接口参数类型
#[derive(Clone)]
pub(crate) enum ParsedInterface {
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
pub(crate) fn process_interface_param(interface: &str) -> InterfaceParamResult { 
    let parsed = interface.parse::<SocketAddr>()
        .map(ParsedInterface::SocketAddr)
        .or_else(|_| interface.parse::<IpAddr>().map(ParsedInterface::Ip))
        .unwrap_or_else(|_| ParsedInterface::Name(interface.to_string()));
    
    match parsed { 
        ParsedInterface::SocketAddr(addr) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(addr.ip(), Some(addr.port()))), 
            is_valid_interface: true, 
            ..InterfaceParamResult::default()
        }, 
        ParsedInterface::Ip(ip) => InterfaceParamResult { 
            interface_ips: Some(interface_ips_from_ip(ip, None)), 
            is_valid_interface: true, 
            ..InterfaceParamResult::default()
        }, 
        ParsedInterface::Name(name) => {
            // 验证接口名是否有效
            let is_valid = is_valid_interface_name(&name);
            
            // 在Windows系统上，尝试获取接口索引
            #[cfg(target_os = "windows")]
            let interface_index = get_interface_index(&name);
            
            InterfaceParamResult { 
                interface_ips: None, 
                is_valid_interface: is_valid,
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                name: Some(name),
                #[cfg(target_os = "windows")]
                interface_index,
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
        // SAFETY: raw_fd 是 TcpSocket 的有效 fd；setsockopt 是标准 POSIX 调用，
        // IP_BIND_ADDRESS_NO_PORT 设置仅影响端口分配策略，失败时不会导致未定义行为。
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
fn create_tcp_socket_for_ip(addr: &IpAddr) -> TcpSocket {
    match addr {
        IpAddr::V4(_) => TcpSocket::new_v4().unwrap(),
        IpAddr::V6(_) => TcpSocket::new_v6().unwrap(),
    }
}

#[cfg(target_os = "linux")]
fn bind_to_interface(sock: &TcpSocket, name: &str) -> Option<()> {
    sock.bind_device(Some(name.as_bytes())).ok()
}

#[cfg(target_os = "macos")]
fn bind_to_interface(sock: &TcpSocket, name: &str, for_addr: IpAddr) -> Option<()> {
    let c_name = std::ffi::CString::new(name).ok()?;
    let idx = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if idx == 0 {
        return None;
    }

    let (level, optname) = match for_addr {
        IpAddr::V4(_) => (libc::IPPROTO_IP, libc::IP_BOUND_IF),
        IpAddr::V6(_) => (libc::IPPROTO_IPV6, libc::IPV6_BOUND_IF),
    };

    let fd = sock.as_raw_fd();
    // SAFETY: fd 有效，idx 合法，level/optname 与 for_addr 协议族匹配。
    unsafe {
        (libc::setsockopt(fd, level, optname, &idx as *const _ as *const _, std::mem::size_of_val(&idx) as libc::socklen_t) == 0)
            .then_some(())
    }
}

/// Windows: 按接口索引绑定
#[cfg(target_os = "windows")]
fn bind_to_interface_index(sock: &TcpSocket, iface_idx: u32, for_addr: IpAddr) -> bool {
    let raw = sock.as_raw_socket();

    let (level, optname, idx_bytes) = match for_addr {
        IpAddr::V4(_) => (IPPROTO_IP, IP_UNICAST_IF, iface_idx.to_be_bytes()),
        IpAddr::V6(_) => (IPPROTO_IPV6, IPV6_UNICAST_IF, iface_idx.to_ne_bytes()),
    };

    let res = unsafe {
        // SAFETY: raw 是 TcpSocket 的有效 socket 句柄；setsockopt 是标准 Windows API，
        // 参数 level/optname 使用 Win32 常量，idx_bytes 是合法字节切片。
        setsockopt(
            raw as _,
            level,
            optname,
            idx_bytes.as_ptr() as *const _,
            idx_bytes.len() as i32,
        )
    };

    res != SOCKET_ERROR
}

/// Windows: 获取接口索引
#[cfg(target_os = "windows")]
pub(crate) fn get_interface_index(name: &str) -> Option<u32> {
    // SAFETY: 遵循 GetAdaptersAddresses 标准两阶段调用模式——先查询缓冲区大小，
    // 分配后再次查询。FriendlyName 指针在 buffer 生命周期内有效。所有 Win32 常量
    // 使用标准值且已验证。循环遍历链表时每次检查 current.is_null()。
    unsafe {
        let mut size: u32 = 0;

        let ret = GetAdaptersAddresses(
            AF_UNSPEC as u32,
            GAA_FLAG_INCLUDE_PREFIX,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        );

        if ret != ERROR_BUFFER_OVERFLOW {
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        let adapters = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;

        let ret = GetAdaptersAddresses(
            AF_UNSPEC as u32,
            GAA_FLAG_INCLUDE_PREFIX,
            std::ptr::null_mut(),
            adapters,
            &mut size,
        );

        if ret != ERROR_SUCCESS {
            return None;
        }

        let mut current = adapters;
        while !current.is_null() {
            let friendly = (*current).FriendlyName;
            if !friendly.is_null() {
                let mut len = 0;
                while *friendly.add(len) != 0 {
                    len += 1;
                }

                let slice = std::slice::from_raw_parts(friendly, len);
                let name_str = String::from_utf16_lossy(slice);

                if name_str == name {
                    return Some((*current).Anonymous1.Anonymous.IfIndex);
                }
            }

            current = (*current).Next;
        }
    }
    None
}

/// 绑定 TCP Socket
pub(crate) async fn bind_socket_to_interface(
    addr: SocketAddr,
    interface_config: &InterfaceParamResult,
) -> Option<TcpSocket> {
    // 创建基础socket
    let sock = create_tcp_socket_for_ip(&addr.ip());
    let _ = sock.set_reuseaddr(true);

    if let Some(ref ips) = interface_config.interface_ips {
        // 如果提供了IP地址，则绑定IP地址
        bind_source_ip_to_socket(&sock, &addr, ips)?;
        return Some(sock);
    }

    // 使用结构体中的接口索引
    #[cfg(target_os = "windows")]
    if let Some(idx) = interface_config.interface_index {
        // 尝试绑定到接口索引
        if !bind_to_interface_index(&sock, idx, addr.ip()) {
            return None;
        }
    }

    #[cfg(target_os = "linux")]
    if let Some(ref name) = interface_config.name {
        bind_to_interface(&sock, name)?;
    }

    #[cfg(target_os = "macos")]
    if let Some(ref name) = interface_config.name {
        bind_to_interface(&sock, name, addr.ip())?;
    }

    Some(sock)
}