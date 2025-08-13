use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use fastrand;
use ipnet::IpNet;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::args::Args;
use crate::common::get_list;

/// IP地址获取结构体
/// 使用全局统一原子计数器作为全体区间索引
pub struct IpBuffer {
    ip_sources: Vec<IpSource>,          // 统一的IP源列表（包括单IP和CIDR）
    global_counter: AtomicUsize,        // 全局统一计数器
    total_count: usize,                 // 总IP数量
    tcp_port: u16,                      // TCP端口
}

/// IP源枚举
/// 统一处理单个IP和CIDR网段
enum IpSource {
    /// 单个IP地址
    Single {
        addr: SocketAddr,       // IP地址
        start_index: usize,     // 在全局索引中的位置
    },
    /// CIDR网段
    Cidr {
        network: IpNet,         // 网络地址
        start_index: usize,     // 在全局索引中的起始位置
        count: usize,           // IP数量
        interval_size: u128,    // 间隔大小
        network_start: u128,    // 网络起始地址
        network_end: u128,      // 网络结束地址
    },
}

impl IpSource {
    /// 根据全局索引生成IP地址
    fn generate_ip(&self, global_index: usize, tcp_port: u16) -> Option<SocketAddr> {
        match self {
            IpSource::Single { addr, start_index } => {
                // 精确匹配该单IP的索引
                if global_index == *start_index {
                    Some(*addr)
                } else {
                    None
                }
            }
            IpSource::Cidr { 
                network, 
                start_index, 
                count, 
                interval_size, 
                network_start, 
                network_end 
            } => {
                // 检查索引是否在该CIDR范围内
                if global_index < *start_index || global_index >= *start_index + *count {
                    return None;
                }
                
                // 计算在该CIDR内的本地索引
                let local_index = global_index - *start_index;
                
                let interval_start = *network_start + (local_index as u128 * *interval_size);
                let interval_end = if local_index == *count - 1 {
                    *network_end
                } else {
                    *network_start + ((local_index + 1) as u128 * *interval_size - 1)
                };

                let random_ip = fastrand::u128(interval_start..=interval_end);

                match network {
                    IpNet::V4(_) => Some(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::from(random_ip as u32)), 
                        tcp_port
                    )),
                    IpNet::V6(_) => Some(SocketAddr::new(
                        IpAddr::V6(Ipv6Addr::from(random_ip)), 
                        tcp_port
                    )),
                }
            }
        }
    }
    
    /// 检查全局索引是否属于该IP源
    fn contains_index(&self, global_index: usize) -> bool {
        match self {
            IpSource::Single { start_index, .. } => {
                global_index == *start_index  // 精确匹配该单IP的索引
            }
            IpSource::Cidr { start_index, count, .. } => {
                global_index >= *start_index && global_index < *start_index + *count
            }
        }
    }
}

impl IpBuffer {
    /// 创建新的IP获取实例
    fn new(ip_sources: Vec<IpSource>, total_count: usize, tcp_port: u16) -> Self {
        Self {
            ip_sources,
            global_counter: AtomicUsize::new(0),
            total_count,
            tcp_port,
        }
    }

    /// 获取一个IP地址
    /// 使用全局统一原子计数器作为全体区间索引
    pub async fn pop(&self) -> Option<SocketAddr> {
        // 全局原子计数，无锁并发
        let global_index = self.global_counter.fetch_add(1, Ordering::Relaxed);
        
        // 检查是否超出总数
        if global_index >= self.total_count {
            return None;
        }
        
        // 根据全局索引找到对应的IP源并生成IP
        for ip_source in &self.ip_sources {
            if ip_source.contains_index(global_index) {
                return ip_source.generate_ip(global_index, self.tcp_port);
            }
        }
        
        None // 未找到对应的IP源
    }

    /// 获取预期总IP数量
    pub fn total_expected(&self) -> usize {
        self.total_count
    }
}

/// 收集所有IP地址来源
/// 包括文本参数、URL链接和文件中的IP地址
async fn collect_ip_sources(ip_text: &str, ip_url: &str, ip_file: &str) -> Vec<String> {
    let mut ip_sources = Vec::new();
    
    // 处理文本参数中的IP地址
    if !ip_text.is_empty() {
        ip_sources.extend(
            ip_text.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        );
    }
    
    // 处理URL链接中的IP地址列表
    if !ip_url.is_empty() {
        let url_list = get_list(ip_url, 5).await;
        ip_sources.extend(url_list.iter().map(|s| s.to_string()));
    }
    
    // 处理文件中的IP地址
    if !ip_file.is_empty() && std::path::Path::new(ip_file).exists() {
        if let Ok(lines) = read_lines(ip_file) {
            for line in lines.flatten() {
                let line = line.trim();
                if !line.is_empty() {
                    ip_sources.push(line.to_string());
                }
            }
        }
    }
    
    // 排序并去重
    ip_sources.sort();
    ip_sources.dedup();
    ip_sources
}

/// 解析IP地址范围
/// 支持格式如：192.168.1.0/24=100 或 192.168.1.1
/// 返回IP部分和自定义数量
fn parse_ip_range(ip_range: &str) -> (String, Option<u128>) {
    let parts: Vec<&str> = ip_range.split('=').collect();
    if parts.len() > 1 {
        let ip_part = parts[0].trim();
        let count_str = parts[1].trim();
        let count = count_str.parse::<u128>()
            .ok()
            .filter(|&n| n > 0)
            .map(|n| n.min(u128::MAX));
        (ip_part.to_string(), count)
    } else {
        (ip_range.to_string(), None)
    }
}

/// 根据配置参数收集所有IP地址并创建IP缓冲区
pub fn load_ip_to_buffer(config: &Args) -> IpBuffer {
    // 收集所有IP地址来源
    let ip_sources = tokio::task::block_in_place(|| {
        let ip_text = &config.ip_text;
        let ip_url = &config.ip_url;
        let ip_file = &config.ip_file;
        tokio::runtime::Handle::current().block_on(collect_ip_sources(&ip_text, &ip_url, &ip_file))
    });
    
    // 如果没有IP地址，直接返回空
    if ip_sources.is_empty() {
        return IpBuffer::new(Vec::new(), 0, config.tcp_port);
    }
    
    let mut single_ips = Vec::new();
    let mut cidr_info = Vec::new();
    let mut total_expected = 0;

    // 遍历所有IP地址来源并进行分类处理
    for ip_range in &ip_sources {
        let (ip_range_str, custom_count) = parse_ip_range(ip_range);
        
        // 处理单个IP地址（带端口）
        if let Ok(socket_addr) = SocketAddr::from_str(&ip_range_str) {
            single_ips.push(socket_addr);
            total_expected += 1;
            continue;
        }
        
        // 处理单个IP地址（不带端口，使用默认端口）
        if let Ok(ip_addr) = IpAddr::from_str(&ip_range_str) {
            single_ips.push(SocketAddr::new(ip_addr, config.tcp_port));
            total_expected += 1;
            continue;
        }

        // 处理CIDR块
        if let Ok(network) = ip_range_str.parse::<IpNet>() {
            // 处理单个IP的CIDR块（/32或/128）
            match &network {
                IpNet::V4(ipv4_net) if ipv4_net.prefix_len() == 32 => {
                    single_ips.push(SocketAddr::new(IpAddr::V4(ipv4_net.addr()), config.tcp_port));
                    total_expected += 1;
                },
                IpNet::V6(ipv6_net) if ipv6_net.prefix_len() == 128 => {
                    single_ips.push(SocketAddr::new(IpAddr::V6(ipv6_net.addr()), config.tcp_port));
                    total_expected += 1;
                },
                _ => {
                    // 计算需要测试的IP数量
                    let count = calculate_ip_count(&ip_range_str, custom_count, config.test_all);
                    let (start, end) = match &network {
                        IpNet::V4(ipv4_net) => {
                            let start = u32::from_be_bytes(ipv4_net.network().octets()) as u128;
                            let end = u32::from_be_bytes(ipv4_net.broadcast().octets()) as u128;
                            (start, end)
                        },
                        IpNet::V6(ipv6_net) => {
                            let start = u128::from_be_bytes(ipv6_net.network().octets());
                            let end = u128::from_be_bytes(ipv6_net.broadcast().octets());
                            (start, end)
                        }
                    };
                    
                    // 如果溢出，就用 u128::MAX
                    let range_size = (end - start).checked_add(1).unwrap_or(u128::MAX);

                    let adjusted_count = count.min(range_size) as usize;

                    // 计算每个区间的间隔大小
                    let interval_size = if adjusted_count > 0 {
                        (range_size / adjusted_count as u128).max(1)
                    } else {
                        1
                    };
                    
                    total_expected += adjusted_count;
                    cidr_info.push((network, adjusted_count, start, end, interval_size));
                }
            }
        }
    }
    
    // 创建统一的IP源列表
    let mut ip_sources = Vec::new();
    let mut current_index = 0;
    
    // 添加单个IP源
    for single_ip in single_ips {
        ip_sources.push(IpSource::Single {
            addr: single_ip,
            start_index: current_index,
        });
        current_index += 1;
    }
    
    // 添加CIDR源
    for (network, count, network_start, network_end, interval_size) in cidr_info {
        ip_sources.push(IpSource::Cidr {
            network,
            start_index: current_index,
            count,
            interval_size,
            network_start,
            network_end,
        });
        current_index += count;
    }
    
    IpBuffer::new(ip_sources, total_expected, config.tcp_port)
}

/// 计算需要测试的IP地址数量
/// 根据IP范围、自定义数量和测试模式计算实际要测试的IP数量
fn calculate_ip_count(ip_range: &str, custom_count: Option<u128>, test_all: bool) -> u128 {
    // 如果是单个IP地址，直接返回1
    if SocketAddr::from_str(ip_range).is_ok() || IpAddr::from_str(ip_range).is_ok() {
        return 1;
    }
    
    // 处理CIDR网络
    if let Ok(network) = ip_range.parse::<IpNet>() {
        let (prefix, is_ipv4) = match network {
            IpNet::V4(ipv4_net) => (ipv4_net.prefix_len(), true),
            IpNet::V6(ipv6_net) => (ipv6_net.prefix_len(), false),
        };

        // 如果有自定义数量，优先使用
        if let Some(count) = custom_count {
            return count;
        }

        // 如果是IPv4且启用全量测试，计算所有IP
        if is_ipv4 && test_all {
            return if prefix < 32 { 2u128.pow((32 - prefix) as u32) } else { 1 };
        }

        // 否则使用采样算法计算数量
        return calculate_sample_count(prefix, is_ipv4);
    }
    
    0
}

/// 计算采样数量
/// 根据网络前缀长度和IP版本计算合理的采样数量
pub fn calculate_sample_count(prefix: u8, is_ipv4: bool) -> u128 {
    let base: u8 = if is_ipv4 { 31 } else { 127 };
    let exp = base.saturating_sub(prefix);
    let clamped_exp = exp.min(16);
    1u128 << clamped_exp
}

/// 读取文件的所有行
/// 返回一个迭代器，用于逐行读取文件内容
fn read_lines(filename: &str) -> io::Result<io::Lines<io::BufReader<File>>> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}