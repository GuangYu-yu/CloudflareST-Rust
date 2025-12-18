use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    Mutex,
    RwLock,
};

use crate::args::Args;

pub enum IpCidr {
    V4(Ipv4Addr, u8),
    V6(Ipv6Addr, u8),
}

impl IpCidr {
    /// 获取前缀长度
    pub fn prefix_len(&self) -> u8 {
        match self {
            IpCidr::V4(_, len) => *len,
            IpCidr::V6(_, len) => *len,
        }
    }

    /// 返回 (start, end)
    pub fn range_u128(&self) -> (u128, u128) {
        match self {
            IpCidr::V4(ip, prefix) => {
                let ip_u32: u32 = (*ip).into();
                let len = *prefix;
                // 处理特殊情况 /0 和 /32
                let mask = if len == 0 {
                    0
                } else if len >= 32 {
                    u32::MAX
                } else {
                    !((1 << (32 - len)) - 1)
                };
                
                let start = ip_u32 & mask;
                let end = start | (!mask);
                (start as u128, end as u128)
            }
            IpCidr::V6(ip, prefix) => {
                let ip_u128: u128 = (*ip).into();
                let len = *prefix;
                let mask = if len == 0 {
                    0
                } else if len >= 128 {
                    u128::MAX
                } else {
                    !((1 << (128 - len)) - 1)
                };
                
                let start = ip_u128 & mask;
                let end = start | (!mask);
                (start, end)
            }
        }
    }
}

impl IpCidr {
    /// 判断是否为单主机 CIDR（/32 或 /128）
    pub fn is_single_host(&self) -> bool {
        matches!(self, IpCidr::V4(_, 32) | IpCidr::V6(_, 128))
    }

    /// 解析 CIDR 字符串
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return None;
        }

        let ip = IpAddr::from_str(parts[0]).ok()?;
        let prefix = parts[1].parse::<u8>().ok()?;

        match ip {
            IpAddr::V4(v4) => {
                if prefix > 32 {
                    return None;
                }
                Some(IpCidr::V4(v4, prefix))
            }
            IpAddr::V6(v6) => {
                if prefix > 128 {
                    return None;
                }
                Some(IpCidr::V6(v6, prefix))
            }
        }
    }
}

/// IP地址获取结构体
/// 用于管理和分发需要测试的IP地址
pub struct IpBuffer {
    total_expected: usize,                           // 预期总IP数量
    cidr_states: Arc<RwLock<Vec<Arc<CidrState>>>>,   // 每个CIDR状态
    single_ips: Arc<Mutex<VecDeque<SocketAddr>>>,     // 单个IP地址列表
    current_cidr: AtomicUsize,                      // 轮询索引
    tcp_port: u16,                                  // TCP端口
}

/// CIDR网络状态结构体
/// 用于管理CIDR网络中的IP地址生成
struct CidrState {
    id: usize,                 // 唯一标识符
    network: IpCidr,             // 网络地址
    total_count: usize,         // 总数量
    interval_size: u128,        // 间隔大小
    start: u128,                // 起始地址
    end: u128,                  // 结束地址
    index_counter: AtomicUsize, // 原子计数器
}

impl CidrState {
    // SplitMix64 混淆常数 
    const MIX_GAMMA: u64 = 0x9E3779B97F4A7C15; 
    const MIX_A: u64 = 0xbf58476d1ce4e5b9;
    const MIX_B: u64 = 0x94d049bb133111eb;

    #[inline(always)] 
    fn splitmix_u64(index: u64, seed_offset: u64) -> u64 { 
        let mut z = index.wrapping_add(seed_offset).wrapping_mul(Self::MIX_GAMMA); 
        
        z = (z ^ (z >> 30)).wrapping_mul(Self::MIX_A); 
        z = (z ^ (z >> 27)).wrapping_mul(Self::MIX_B); 
        
        z ^ (z >> 31) 
    }

    /// 创建新的CIDR状态实例
    fn new(id: usize, network: IpCidr, count: usize, start: u128, end: u128, interval_size: u128) -> Self {
        Self {
            id,
            network,
            total_count: count,
            interval_size,
            start,
            end,
            index_counter: AtomicUsize::new(0),
        }
    }

    /// 生成下一个随机IP地址
    fn next_ip(&self, tcp_port: u16) -> Option<SocketAddr> {
        let current_index = self.index_counter.fetch_add(1, Ordering::Relaxed);

        if current_index >= self.total_count {
            return None;
        }

        let interval_start = self.start + (current_index as u128 * self.interval_size);
        
        let actual_interval_size = if current_index == self.total_count - 1 {
            (self.end - interval_start).saturating_add(1)
        } else {
            self.interval_size
        };

        let random_offset = if actual_interval_size <= 1 {
            0
        } else {
            let mixed_val = Self::splitmix_u64(
                current_index as u64,
                self.id as u64
            );
            
            (mixed_val as u128) % actual_interval_size
        };

        let random_ip = interval_start + random_offset;

        match self.network {
            IpCidr::V4(_, _) => Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(random_ip as u32)),
                tcp_port,
            )),
            IpCidr::V6(_, _) => Some(SocketAddr::new(
                IpAddr::V6(Ipv6Addr::from(random_ip)),
                tcp_port,
            )),
        }
    }
}

impl IpBuffer {
    /// 创建新的IP获取实例
    fn new(
        cidr_states: Vec<CidrState>,
        single_ips: Vec<SocketAddr>,
        total_expected: usize,
        tcp_port: u16,
    ) -> Self {
        // 将每个CIDR状态包装成Arc
        let cidr_states: Vec<Arc<CidrState>> = cidr_states
            .into_iter()
            .map(Arc::new)
            .collect();

        // 创建IP列表
        let single_ip_list = Arc::new(Mutex::new(VecDeque::from(single_ips)));

        Self {
            total_expected,
            cidr_states: Arc::new(RwLock::new(cidr_states)),
            single_ips: single_ip_list,
            current_cidr: AtomicUsize::new(0),
            tcp_port,
        }
    }

    /// 获取一个IP地址
    /// 优先返回单个IP，再轮询CIDR
    pub fn pop(&self) -> Option<SocketAddr> {
        // 1. 单 IP 列表
        if let Ok(mut ips) = self.single_ips.lock() 
            && let Some(ip) = ips.pop_front() 
        { 
            return Some(ip); 
        } 

        // 2. CIDR 轮询
        loop { 
            // 获取一个 CIDR 引用（读锁阶段）
            let (cidr_id, cidr_arc) = { 
                let states = self.cidr_states.read().ok()?; 
                if states.is_empty() { 
                    return None; 
                } 
                let idx = self.current_cidr.fetch_add(1, Ordering::Relaxed) % states.len(); 
                let s = &states[idx]; 
                (s.id, s.clone()) 
            }; 

            // 释放锁后尝试获取 IP
            if let Some(ip) = cidr_arc.next_ip(self.tcp_port) { 
                return Some(ip); 
            } 

            // 3. 耗尽则按 id 移除（写锁阶段）
            let mut states = self.cidr_states.write().ok()?; 
            if let Some(pos) = states.iter().position(|s| s.id == cidr_id) { 
                states.swap_remove(pos); 
            } 
        } 
    }

    /// 获取预期总IP数量
    pub fn total_expected(&self) -> usize {
        self.total_expected
    }
}

/// 收集所有IP地址来源
/// 包括文本参数、URL链接和文件中的IP地址
async fn collect_ip_sources(ip_text: &str, ip_url: &str, ip_file: &str) -> Vec<String> {
    let ip_sources = {
        let mut sources = Vec::new();

        // 处理文本参数中的IP地址
        if !ip_text.is_empty() {
            sources.extend(
                ip_text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string),
            );
        }

        // 处理URL链接中的IP地址列表
        if !ip_url.is_empty() {
            // 内联 get_list 逻辑
            let test_url = if ip_url.contains("://") { ip_url.to_string() } else { format!("https://{}", ip_url) };
            
            // 解析URL获取URI和主机名
            if let Some((uri, host)) = crate::hyper::parse_url_to_uri(&test_url) {
                let mut url_list = Vec::new();
                // 最多尝试5次
                for i in 1..=5 {
                    // 创建客户端
                    let mut client = match crate::hyper::client_builder() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    // 发送 GET 请求
                    if let Ok(body_bytes) = crate::hyper::send_get_request_simple(&mut client, &host, uri.clone(), 5000).await {
                        let content = String::from_utf8_lossy(&body_bytes);
                        url_list = content
                            .lines()
                            .map(|line| line.trim())
                            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('#'))
                            .map(|line| line.to_string())
                            .collect();
                        break;
                    }

                    // 重试提示
                    if i < 5 {
                        crate::warning_println(format_args!("列表请求失败，正在第{}次重试..", i));
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    } else {
                        crate::warning_println(format_args!("获取列表已达到最大重试次数"));
                    }
                }
                
                sources.extend(url_list);
            }
        }

        // 处理文件中的IP地址
        if !ip_file.is_empty() && std::path::Path::new(ip_file).exists()
            && let Ok(lines) = read_lines(ip_file) {
            for line in lines.map_while(Result::ok) {
                let line = line.trim();
                if !line.is_empty() {
                    sources.push(line.to_string());
                }
            }
        }

        sources
    };

    // 排序并去重
    let mut sorted_sources = ip_sources;
    sorted_sources.sort();
    sorted_sources.dedup();
    sorted_sources
}

/// 解析IP地址范围
/// 支持格式如：192.168.1.0/24=100 或 192.168.1.1
/// 返回IP部分和自定义数量
fn parse_ip_range(ip_range: &str) -> (String, Option<u128>) {
    let (ip_part, count) = {
        let parts: Vec<&str> = ip_range.split('=').collect();
        if parts.len() > 1 {
            let ip_part = parts[0].trim();
            let count_str = parts[1].trim();
            let count = count_str
                .parse::<u128>()
                .ok()
                .filter(|&n| n > 0);
            (ip_part, count)
        } else {
            (ip_range, None)
        }
    };

    (ip_part.to_string(), count)
}

/// IP地址解析结果枚举
enum IpParseResult {
    /// 单个IP地址（带端口）
    SocketAddr(SocketAddr),
    /// 单个IP地址（不带端口）
    IpAddr(IpAddr),
    /// CIDR网络
    Network(IpCidr),
    /// 解析失败
    Invalid,
}

/// 解析IP地址字符串
/// 返回解析结果，包括单个IP地址（带/不带端口）或CIDR网络
fn parse_ip_with_port(ip_str: &str) -> IpParseResult {
    // 尝试解析为带端口的IP地址
    if let Ok(socket_addr) = SocketAddr::from_str(ip_str) {
        return IpParseResult::SocketAddr(socket_addr);
    }

    // 尝试解析为不带端口的IP地址
    if let Ok(ip_addr) = IpAddr::from_str(ip_str) {
        return IpParseResult::IpAddr(ip_addr);
    }

    // 尝试解析为CIDR网络
    if let Some(network) = IpCidr::parse(ip_str) {
        return IpParseResult::Network(network);
    }

    // 解析失败
    IpParseResult::Invalid
}

/// 根据配置参数收集所有IP地址并创建IP缓冲区
pub async fn load_ip_to_buffer(config: &Args) -> IpBuffer {
    // 收集所有IP地址来源
    let ip_sources = collect_ip_sources(&config.ip_text, &config.ip_url, &config.ip_file).await;

    // 如果没有IP地址，直接返回空
    if ip_sources.is_empty() {
        return IpBuffer::new(Vec::new(), Vec::new(), 0, config.tcp_port);
    }

    let (single_ips, cidr_info, total_expected) = {
        let mut single_ips = Vec::new();
        let mut cidr_info = Vec::new();
        let mut total_expected = 0;

        // 遍历所有IP地址来源并进行分类处理
        for ip_range in &ip_sources {
            let (ip_range_str, custom_count) = parse_ip_range(ip_range);

            // 使用统一的IP解析函数
            match parse_ip_with_port(&ip_range_str) {
                IpParseResult::SocketAddr(socket_addr) => {
                    single_ips.push(socket_addr);
                    total_expected += 1;
                }
                IpParseResult::IpAddr(ip_addr) => {
                    single_ips.push(SocketAddr::new(ip_addr, config.tcp_port));
                    total_expected += 1;
                }
                IpParseResult::Network(network) => {
                    // 处理单个IP的CIDR块（/32或/128）
                    if network.is_single_host() {
                        // 如果是单IP掩码，直接取起始地址当作单IP
                        let (start, _) = network.range_u128();
                        let ip = match network {
                            IpCidr::V4(_, _) => IpAddr::V4(Ipv4Addr::from(start as u32)),
                            IpCidr::V6(_, _) => IpAddr::V6(Ipv6Addr::from(start)),
                        };
                        single_ips.push(SocketAddr::new(ip, config.tcp_port));
                        total_expected += 1;
                    } else {
                        // 计算需要测试的IP数量
                        let count = calculate_ip_count(&ip_range_str, custom_count, config.test_all_ipv4);
                        let (start, end) = network.range_u128();

                        // 如果溢出，就用 u128::MAX
                        let range_size = (end - start).saturating_add(1);

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
                IpParseResult::Invalid => {
                    // 忽略无效的IP地址
                }
            }
        }

        (single_ips, cidr_info, total_expected)
    };

    // 创建CIDR状态列表
    let cidr_states: Vec<_> = cidr_info
        .into_iter()
        .enumerate()
        .map(|(id, (net, count, start, end, size))| CidrState::new(id, net, count, start, end, size))
        .collect();

    IpBuffer::new(cidr_states, single_ips, total_expected, config.tcp_port)
}

/// 计算需要测试的IP地址数量
/// 根据IP范围、自定义数量和测试模式计算实际要测试的IP数量
fn calculate_ip_count(ip_range: &str, custom_count: Option<u128>, test_all_ipv4: bool) -> u128 {
    // 使用统一的IP解析函数
    match parse_ip_with_port(ip_range) {
        IpParseResult::SocketAddr(_) | IpParseResult::IpAddr(_) => {
            // 单个IP地址，直接返回1
            1
        }
        IpParseResult::Network(network) => {
            let prefix = network.prefix_len();
            let is_ipv4 = matches!(network, IpCidr::V4(_, _));

            // 如果有自定义数量，优先使用
            if let Some(count) = custom_count {
                return count;
            }

            // 如果是IPv4且启用全量测试，计算所有IP
            if is_ipv4 && test_all_ipv4 {
                return if prefix < 32 {
                    2u128.pow((32 - prefix) as u32)
                } else {
                    1
                };
            }

            // 否则使用采样算法计算数量
            calculate_sample_count(prefix, is_ipv4)
        }
        IpParseResult::Invalid => {
            // 无效的IP地址，返回0
            0
        }
    }
}

/// 计算采样数量
/// 根据网络前缀长度和IP版本计算合理的采样数量
pub fn calculate_sample_count(prefix: u8, is_ipv4: bool) -> u128 {
    let base: u8 = if is_ipv4 { 31 } else { 127 };
    let exp = base.saturating_sub(prefix);
    let clamped_exp = exp.min(18);
    1u128 << clamped_exp.saturating_sub(3)
}

/// 读取文件的所有行
/// 返回一个迭代器，用于逐行读取文件内容
fn read_lines(filename: &str) -> io::Result<io::Lines<io::BufReader<File>>> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}