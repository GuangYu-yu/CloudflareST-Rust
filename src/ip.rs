use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::str::FromStr;
use rand::Rng;
use ipnet::IpNet;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::args::Args;
use crate::common::get_list;

pub struct IpBuffer {
    ip_receiver: Receiver<SocketAddr>,
    ip_sender: Option<Sender<()>>,
    total_expected: usize,
    producer_active: Arc<AtomicBool>,
}

// CIDR状态跟踪结构体
struct CidrState {
    network: IpNet,
    current_index: usize,
    total_count: usize,
    interval_size: u128,
    start: u128,
    end: u128,
}

impl CidrState {
    fn new(network: IpNet, count: usize, start: u128, end: u128, interval_size: u128) -> Self {
        Self {
            network,
            current_index: 0,
            total_count: count,
            interval_size,
            start,
            end,
        }
    }

    // 生成下一个IP
    fn next_ip(&mut self, rng: &mut impl Rng, tcp_port: u16) -> Option<SocketAddr> {
        if self.current_index >= self.total_count {
            return None;
        }

        // 计算当前区间的起始和结束
        let interval_start = self.start + (self.current_index as u128 * self.interval_size);
        let interval_end = if self.current_index == self.total_count - 1 {
            self.end // 最后一个区间使用广播地址作为结束
        } else {
            self.start + ((self.current_index + 1) as u128 * self.interval_size - 1)
        };

        // 在当前区间内随机选择一个IP
        let random_ip = rng.random_range(interval_start..=interval_end);
        
        // 增加索引，为下次生成做准备
        self.current_index += 1;

        // 根据网络类型转换IP地址
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
    // 创建默认的空缓冲区
    fn new(ip_rx: Receiver<SocketAddr>, req_tx: Option<Sender<()>>, producer_active: Arc<AtomicBool>) -> Self {
        Self {
            ip_receiver: ip_rx,
            ip_sender: req_tx,
            total_expected: 0,
            producer_active,
        }
    }

    // 从缓存获取下一个IP
    pub fn pop(&mut self) -> Option<SocketAddr> {
        // 如果生产者活动，尝试从通道获取新IP
        if self.producer_active.load(Ordering::Relaxed) {
            // 发送单个请求信号
            if let Some(sender) = &self.ip_sender {
                let _ = sender.try_send(());  // 每次只发送一个请求信号
            }
            
            // 尝试从通道接收IP
            match self.ip_receiver.try_recv() {
                Ok(ip) => Some(ip),
                Err(_) => {
                    // 如果没有立即可用的IP，但生产者仍在活动，则阻塞等待
                    if self.producer_active.load(Ordering::Relaxed) {
                        self.ip_receiver.recv().ok()
                    } else {
                        None
                    }
                }
            }
        } else {
            // 生产者已不活动，尝试最后一次非阻塞接收
            self.ip_receiver.try_recv().ok()
        }
    }

    // 获取预计总IP数量
    pub fn total_expected(&self) -> usize {
        self.total_expected
    }

    // 判断是否已读取完所有IP
    pub fn is_empty(&self) -> bool {
        !self.producer_active.load(Ordering::Relaxed) && 
        self.ip_receiver.is_empty()
    }
}

// 收集IP源
async fn collect_ip_sources(ip_text: &str, ip_url: &str, ip_file: &str) -> Vec<String> {
    let mut ip_sources = Vec::new();
    
    // 1. 从参数中获取IP段数据
    if !ip_text.is_empty() {
        ip_sources.extend(
            ip_text.split(',')
                .map(|ip| ip.trim())
                .filter(|ip| !ip.is_empty())
                .map(|ip| ip.to_string())
        );
    }
    
    // 2. 从URL获取IP段数据
    if !ip_url.is_empty() {
        let url_list = get_list(ip_url, 3).await;
        ip_sources.extend(url_list);
    }
    
    // 3. 从文件中获取IP段数据
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
    
    // 去重
    ip_sources.sort();
    ip_sources.dedup();
    
    ip_sources
}

// 自定义采样数量
fn parse_ip_range(ip_range: &str) -> (String, Option<u128>) {
    let parts: Vec<&str> = ip_range.split('=').collect();
    if parts.len() > 1 {
        let ip_part = parts[0].trim();
        let count_str = parts[1].trim();
        
        // 尝试解析数量
        let count = count_str.parse::<u128>()
            .ok()
            .filter(|&n| n > 0)
            .map(|n| n.min(u128::MAX));
        
        (ip_part.to_string(), count)
    } else {
        (ip_range.to_string(), None)
    }
}

// 加载IP列表到缓存
pub fn load_ip_to_buffer(config: &Args) -> IpBuffer {
    // 缓冲区大小
    let buffer_size = 4096;
    let (ip_tx, ip_rx) = bounded::<SocketAddr>(buffer_size);
    let (req_tx, req_rx) = bounded::<()>(buffer_size);
    
    let producer_active = Arc::new(AtomicBool::new(true));
    
    // 创建IP缓冲区
    let mut ip_buffer = IpBuffer::new(ip_rx, Some(req_tx), producer_active.clone());
    
    // 克隆需要在线程中使用的数据
    let ip_text = config.ip_text.clone();
    let ip_url = config.ip_url.clone();
    let ip_file = config.ip_file.clone();
    let test_all = config.test_all;
    
    // 使用当前运行时执行异步操作
    let ip_sources = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(collect_ip_sources(&ip_text, &ip_url, &ip_file))
    });
    
    // 如果没有收集到任何IP源，返回空缓冲区
    if ip_sources.is_empty() {
        producer_active.store(false, Ordering::Relaxed);
        return ip_buffer;
    }
    
    // 先计算总IP数量
    let mut total_expected = 0;
    let mut cidr_info = Vec::new();

    for ip_range in &ip_sources {
        // 解析IP范围和自定义数量
        let (ip_range_str, custom_count) = parse_ip_range(ip_range);
        
        // 先尝试解析为SocketAddr
        if let Ok(socket_addr) = SocketAddr::from_str(&ip_range_str) {
            let _ = ip_tx.send(socket_addr);
            total_expected += 1;
            continue;
        }
        
        // 尝试解析为纯IP地址
        if let Ok(ip_addr) = IpAddr::from_str(&ip_range_str) {
            let socket_addr = SocketAddr::new(ip_addr, config.tcp_port);
            let _ = ip_tx.send(socket_addr);
            total_expected += 1;
            continue;
        }

        // 如果前面都失败，再尝试解析为CIDR格式
        if let Ok(network) = ip_range_str.parse::<IpNet>() {
            let count = calculate_ip_count(&network.to_string(), custom_count, test_all);
            
            // 计算start和end
            let (start, end) = match &network {
                IpNet::V4(ipv4_net) => {
                    let network_addr = ipv4_net.network();
                    let broadcast_addr = ipv4_net.broadcast();
                    let start = u32::from_be_bytes(network_addr.octets()) as u128;
                    let end = u32::from_be_bytes(broadcast_addr.octets()) as u128;
                    (start, end)
                },
                IpNet::V6(ipv6_net) => {
                    let network_addr = ipv6_net.network();
                    let broadcast_addr = ipv6_net.broadcast();
                    let start = u128::from_be_bytes(network_addr.octets());
                    let end = u128::from_be_bytes(broadcast_addr.octets());
                    (start, end)
                }
            };
            
            let range_size = end - start + 1;
            let adjusted_count = count.min(range_size);
            
            let interval_size = if adjusted_count > 0 {
                (range_size / adjusted_count).max(1)
            } else {
                1
            };
            
            total_expected += adjusted_count as usize;
            cidr_info.push((network, adjusted_count as usize, start, end, interval_size));
        }
    }
    
    // 设置预计总IP数量
    ip_buffer.total_expected = total_expected;
    
    // 启动生产者线程来处理IP
    let tcp_port = config.tcp_port;
    thread::spawn(move || {
        process_ip_sources_with_cidr_info(
            cidr_info, 
            ip_tx, 
            req_rx, 
            producer_active,
            tcp_port
        );
    });
    
    ip_buffer
}

fn process_ip_sources_with_cidr_info(
    cidr_info: Vec<(IpNet, usize, u128, u128, u128)>,
    ip_tx: Sender<SocketAddr>,
    req_rx: Receiver<()>,
    producer_active: Arc<AtomicBool>,
    tcp_port: u16
) {
    // 创建随机数生成器
    let mut rng = rand::rng();
    
    // 处理所有IP源，创建CIDR状态列表
    let mut cidr_states = Vec::new();
    
    for (network, ip_count, start, end, interval_size) in cidr_info {
        // 处理CIDR格式
        match network {
            IpNet::V4(ipv4_net) if ipv4_net.prefix_len() == 32 => {
                let addr = SocketAddr::new(IpAddr::V4(ipv4_net.addr()), tcp_port);
                let _ = ip_tx.send(addr);
                continue;
            },
            IpNet::V6(ipv6_net) if ipv6_net.prefix_len() == 128 => {
                let addr = SocketAddr::new(IpAddr::V6(ipv6_net.addr()), tcp_port);
                let _ = ip_tx.send(addr);
                continue;
            },
            _ => {
                if ip_count > 0 {
                    cidr_states.push(CidrState::new(network, ip_count, start, end, interval_size));
                }
            }
        }
    }
    
    // 轮询生成IP
    let mut current_index = 0;
    let mut remaining_ips = cidr_states.iter().map(|state| state.total_count).sum::<usize>();
    
    while remaining_ips > 0 && !cidr_states.is_empty() {
        // 等待请求信号
        if req_rx.recv().is_err() {
            break;
        }
        
        // 确保索引在有效范围内
        if current_index >= cidr_states.len() {
            current_index = 0;
        }
        
        // 获取当前CIDR状态
        let state = &mut cidr_states[current_index];
        
        // 生成下一个IP
        if let Some(addr) = state.next_ip(&mut rng, tcp_port) {
            if ip_tx.send(addr).is_err() {
                break;
            }
            remaining_ips -= 1;
            
            // 检查当前CIDR是否已经生成完所有IP
            if state.current_index >= state.total_count {
                // 移除当前CIDR
                cidr_states.remove(current_index);
                continue;
            }
        }
        
        // 移动到下一个CIDR
        current_index = (current_index + 1) % cidr_states.len();
    }
    
    // 标记生产者已完成
    producer_active.store(false, Ordering::Relaxed);
}

// 获取给定IP范围的采样数量
fn calculate_ip_count(ip_range: &str, custom_count: Option<u128>, test_all: bool) -> u128 {
    // 尝试解析为SocketAddr或单个IP
    if SocketAddr::from_str(ip_range).is_ok() || IpAddr::from_str(ip_range).is_ok() {
        return 1;
    }
    
    // 如果不是单个IP，再尝试解析为CIDR
    if let Ok(network) = ip_range.parse::<IpNet>() {
        match network {
            IpNet::V4(ipv4_net) => {
                let prefix = ipv4_net.prefix_len();
                if test_all {
                    return if prefix < 32 { 2u128.pow((32 - prefix) as u32) } else { 1 };
                } else if let Some(count) = custom_count {
                    // 使用自定义数量
                    return count;
                } else {
                    return calculate_sample_count(prefix, true);
                }
            }
            IpNet::V6(ipv6_net) => {
                let prefix = ipv6_net.prefix_len();
                if let Some(count) = custom_count {
                    // 使用自定义数量
                    return count;
                } else {
                    return calculate_sample_count(prefix, false);
                }
            }
        }
    }
    
    // 无法解析的情况，返回0
    0
}

// 采样数量
pub fn calculate_sample_count(prefix: u8, is_ipv4: bool) -> u128 {
    // IPv4 和 IPv6 的采样数量数组
    static SAMPLES: [u16; 19] = [
        1, 2, 4, 8, 16, 48, 96, 200, 400, 800,
        1600, 1800, 2000, 3000, 4000, 6000,
        10000, 30000, 50000
    ];
    
    let base = if is_ipv4 { 31_usize } else { 127_usize };
    let index = base.saturating_sub(prefix as usize);
    if index > 18 { 80000 } else { u128::from(SAMPLES[index]) }
}

// 读取文件行
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())}
