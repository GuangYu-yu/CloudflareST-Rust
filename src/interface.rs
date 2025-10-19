use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use std::net::IpAddr;

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
        #[cfg(target_os = "linux")]
        {
            // Linux 系统：检查接口是否存在，但保留接口名，不获取 IP 地址
            let is_valid = NetworkInterface::show().map_or(false, |ints| {
                ints.iter().any(|iface| iface.name == interface)
            });

            InterfaceParamResult {
                interface_ips: None,
                is_valid_interface: is_valid,
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            // 非 Linux 系统：尝试获取接口的 IP 地址
            let interface_ips = get_interface_ip(interface);
            let is_valid = interface_ips.is_some();

            InterfaceParamResult {
                interface_ips,
                is_valid_interface: is_valid,
            }
        }
    }
}