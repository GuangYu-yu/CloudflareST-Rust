use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use fastrand;
use ipnet::IpNet;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use crate::args::Args;
use crate::common::get_list;

/// IP地址获取结构体
/// 用于管理和分发需要测试的IP地址
pub struct IpBuffer {
    total_expected: usize,                      // 预期总IP数量
    cidr_states: tokio::sync::Mutex<Vec<CidrState>>, // CIDR状态列表
    single_ips: tokio::sync::Mutex<Vec<SocketAddr>>, // 单个IP地址列表
    tcp_port: u16,                             // TCP端口
}

/// CIDR网络状态结构体
/// 用于管理CIDR网络中的IP地址生成
struct CidrState {
    network: IpNet,         // 网络地址
    total_count: usize,     // 总数量
    interval_size: u128,    // 间隔大小
    start: u128,            // 起始地址
    end: u128,              // 结束地址
    index_counter: Arc<AtomicUsize>, // 原子计数器，用于轮询
}

impl CidrState {
    /// 创建新的CIDR状态实例
    fn new(network: IpNet, count: usize, start: u128, end: u128, interval_size: u128, index_counter: Arc<AtomicUsize>) -> Self {
        Self {
            network,
            total_count: count,
            interval_size,
            start,
            end,
            index_counter,
        }
    }

    /// 生成下一个随机IP地址
    /// 根据当前索引在指定区间内生成随机IP
    fn next_ip(&mut self, tcp_port: u16) -> Option<SocketAddr> {
        let current_index = self.index_counter.fetch_add(1, Ordering::Relaxed);
        
        if current_index >= self.total_count {
            return None;
        }

        let interval_start = self.start + (current_index as u128 * self.interval_size);
        let interval_end = if current_index == self.total_count - 1 {
            self.end
        } else {
            self.start + ((current_index + 1) as u128 * self.interval_size - 1)
        };

        let random_ip = fastrand::u128(interval_start..=interval_end);

        match self.network {
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

impl IpBuffer {
    /// 创建新的IP获取实例
    fn new(cidr_states: Vec<CidrState>, single_ips: Vec<SocketAddr>, total_expected: usize, tcp_port: u16) -> Self {
        Self {
            total_expected,
            cidr_states: tokio::sync::Mutex::new(cidr_states),
            single_ips: tokio::sync::Mutex::new(single_ips),
            tcp_port,
        }
    }

    /// 获取一个IP地址
    /// 异步版本，使用tokio锁
    pub async fn pop(&self) -> Option<SocketAddr> {
        // 优先返回单个IP
        {
            let mut single_ips = self.single_ips.lock().await;
            if let Some(ip) = single_ips.pop() {
                return Some(ip);
            }
        }
        
        // 轮询CIDR块生成IP
        let mut cidr_states = self.cidr_states.lock().await;
        let i = 0;
        while i < cidr_states.len() {
            if let Some(ip) = cidr_states[i].next_ip(self.tcp_port) {
                return Some(ip);
            } else {
                // 如果CIDR块已用完，移除它
                cidr_states.remove(i);
                // 不需要增加i，因为remove后元素会前移
            }
        }
        
        None
    }

    /// 获取预期总IP数量
    pub fn total_expected(&self) -> usize {
        self.total_expected
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
        let ip_text = config.ip_text.clone();
        let ip_url = config.ip_url.clone();
        let ip_file = config.ip_file.clone();
        tokio::runtime::Handle::current().block_on(collect_ip_sources(&ip_text, &ip_url, &ip_file))
    });
    
    // 如果没有IP地址，直接返回空
    if ip_sources.is_empty() {
        return IpBuffer::new(Vec::new(), Vec::new(), 0, config.tcp_port);
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
    
    // 创建CIDR状态列表
    let cidr_states: Vec<_> = cidr_info.into_iter()
        .map(|(net, count, start, end, size)| {
            let index_counter = Arc::new(AtomicUsize::new(0));
            CidrState::new(net, count, start, end, size, index_counter)
        })
        .collect();
    
    IpBuffer::new(cidr_states, single_ips, total_expected, config.tcp_port)
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
