use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::str::FromStr;
use rand::Rng;
use ipnet::IpNet;
use reqwest;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::args::Args;
use crate::common::USER_AGENT;  // 导入常量

pub struct IpBuffer {
    ip_receiver: Receiver<IpAddr>,       // 接收IP的通道
    ip_sender: Option<Sender<()>>,       // 发送请求新IP的信号通道
    total_expected: usize,               // 预计总IP数量
    producer_active: Arc<AtomicBool>,    // 生产者是否仍在活动
}

impl IpBuffer {
    // 创建默认的空缓冲区
    fn new(ip_rx: Receiver<IpAddr>, req_tx: Option<Sender<()>>, producer_active: Arc<AtomicBool>) -> Self {
        Self {
            ip_receiver: ip_rx,
            ip_sender: req_tx,
            total_expected: 0,
            producer_active,
        }
    }

    // 从缓存获取下一个IP
    pub fn pop(&mut self) -> Option<IpAddr> {
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
    
    // 设置预计总IP数量
    pub fn set_total_expected(&mut self, count: usize) {
        self.total_expected = count;
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
        let ips: Vec<&str> = ip_text.split(',').collect();
        for ip in ips {
            let ip = ip.trim();
            if !ip.is_empty() {
                ip_sources.push(ip.to_string());
            }
        }
    }
    
    // 2. 从URL获取IP段数据
    if !ip_url.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        
        for i in 1..=3 {
            match client.get(ip_url)
                .header("User-Agent", USER_AGENT)
                .send()
                .await 
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(text) = resp.text().await {
                        for line in text.lines() {
                            let line = line.trim();
                            if !line.is_empty() {
                                ip_sources.push(line.to_string());
                            }
                        }
                        break;
                    }
                }
                _ => {
                    if i < 3 {
                        println!("从 URL 获取 IP 或 CIDR 列表失败，正在重试 ({}/{})", i, 3);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    } else {
                        println!("从 URL 获取 IP 或 CIDR 列表失败，已达到最大重试次数");
                    }
                }
            }
        }
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

// 检查是否为注释行并解析IP范围
fn parse_ip_range_with_comment_check(ip_range: &str) -> Option<(String, Option<usize>)> {
    // 忽略注释行
    if ip_range.starts_with('#') || ip_range.starts_with("//") {
        return None;
    }
    
    // 解析IP范围和自定义数量
    let (ip_range_str, custom_count) = parse_ip_range(ip_range);
    Some((ip_range_str, custom_count))
}

// 加载IP列表到缓存
pub fn load_ip_to_buffer(config: &Args) -> IpBuffer {
    // 缓冲区大小
    let buffer_size = 4096;
    let (ip_tx, ip_rx) = bounded::<IpAddr>(buffer_size);
    let (req_tx, req_rx) = bounded::<()>(buffer_size);
    
    let producer_active = Arc::new(AtomicBool::new(true));
    let producer_active_clone = producer_active.clone();
    
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
        return IpBuffer::new(bounded(0).1, None, Arc::new(AtomicBool::new(false)));
    }
    
    // 先计算总IP数量
    let mut total_expected = 0;
    
    for ip_range in &ip_sources {
        // 检查注释并解析IP范围
        if let Some((ip_range_str, custom_count)) = parse_ip_range_with_comment_check(ip_range) {
            // 使用已解析的结果计算IP数量
            total_expected += calculate_ip_count(&ip_range_str, custom_count, test_all);
        }
    }
    
    // 设置预计总IP数量
    ip_buffer.set_total_expected(total_expected);
    
    // 启动生产者线程来处理IP
    thread::spawn(move || {
        // 处理所有IP源
        for ip_range in ip_sources {
            process_ip_range_to_channel(&ip_range, test_all, &ip_tx, &req_rx);
        }
        
        // 标记生产者已完成
        producer_active_clone.store(false, Ordering::Relaxed);
    });
    
    ip_buffer
}

// 自定义采样数量
fn parse_ip_range(ip_range: &str) -> (String, Option<usize>) {
    let parts: Vec<&str> = ip_range.split('=').collect();
    if parts.len() > 1 {
        let ip_part = parts[0].trim();
        let count_str = parts[1].trim();
        
        // 尝试解析数量
        let count = match count_str.parse::<usize>() {
            Ok(n) if n > 0 => Some(n.min(u32::MAX as usize)),
            _ => None
        };
        
        (ip_part.to_string(), count)
    } else {
        (ip_range.to_string(), None)
    }
}

// 计算给定IP范围的采样数量
fn calculate_ip_count(ip_range: &str, custom_count: Option<usize>, test_all: bool) -> usize {
    // 先尝试解析为单个IP
    if IpAddr::from_str(ip_range).is_ok() {
        return 1;
    }
    
    // 如果不是单个IP，再尝试解析为CIDR
    if let Ok(network) = ip_range.parse::<IpNet>() {
        match network {
            IpNet::V4(ipv4_net) => {
                let prefix = ipv4_net.prefix_len();
                if test_all {
                    return if prefix < 32 { 2u32.pow((32 - prefix) as u32) as usize } else { 1 };
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

// 处理IP范围并发送到通道
fn process_ip_range_to_channel(ip_range: &str, test_all: bool, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>) {
    // 检查注释并解析IP范围
    let Some((ip_range_str, custom_count)) = parse_ip_range_with_comment_check(ip_range) else {
        return;
    };
    
    // 处理CIDR格式的IP段
    if let Ok(network) = ip_range_str.parse::<IpNet>() {
        // 直接处理单IP的CIDR格式（/32或/128）
        match network {
            IpNet::V4(ipv4_net) if ipv4_net.prefix_len() == 32 => {
                let _ = ip_tx.send(IpAddr::V4(ipv4_net.addr()));
                return;
            },
            IpNet::V6(ipv6_net) if ipv6_net.prefix_len() == 128 => {
                let _ = ip_tx.send(IpAddr::V6(ipv6_net.addr()));
                return;
            },
            _ => {
                // 处理其他CIDR格式
                match network {
                    IpNet::V4(_) => stream_ipv4_to_channel(&network, test_all, ip_tx, req_rx, custom_count),
                    IpNet::V6(_) => stream_ipv6_to_channel(&network, ip_tx, req_rx, custom_count),
                }
            }
        }
    }
}

// 流式处理IPv4地址并发送到通道
fn stream_ipv4_to_channel(network: &IpNet, test_all: bool, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>, custom_count: Option<usize>) {
    if let IpNet::V4(ipv4_net) = network {
        if test_all {
            // 使用 hosts() 方法直接获取所有 IP 地址
            let all_ips: Vec<Ipv4Addr> = ipv4_net.hosts().collect();
            for ip_addr in all_ips {
                // 等待请求信号
                if req_rx.recv().is_err() {
                    return;
                }
                // 发送IP
                if ip_tx.send(IpAddr::V4(ip_addr)).is_err() {
                    return;
                }
            }
        } else {
            let prefix = ipv4_net.prefix_len();
            // 使用自定义数量或计算采样数量
            let ip_count = custom_count.unwrap_or_else(|| calculate_sample_count(prefix, true));

            // 创建一个随机数生成器实例
            let mut rng = rand::rng();

            // 直接枚举所有IP再随机采样
            if prefix >= 23 && custom_count.is_none() {
                // 使用 hosts() 方法直接获取所有 IP 地址
                let all_ips: Vec<Ipv4Addr> = ipv4_net.hosts().collect();
                use rand::seq::SliceRandom;
                let sample_count = ip_count.min(all_ips.len());
                let mut sampled = all_ips;
                sampled.shuffle(&mut rng);
                
                // 只保留需要的IP，释放多余内存
                let sampled = sampled.into_iter().take(sample_count).collect::<Vec<_>>();
                
                // 严格按照请求信号发送IP
                for ip in sampled {
                    // 等待请求信号
                    if req_rx.recv().is_err() {
                        return;
                    }
                    // 发送IP
                    if ip_tx.send(IpAddr::V4(ip)).is_err() {
                        return;
                    }
                }
            } else {
                // 其他范围直接随机生成，不去重
                let mut sent_count = 0;
                
                while sent_count < ip_count {
                    // 等待请求信号
                    if req_rx.recv().is_err() {
                        return;
                    }
                    // 传递随机数生成器的引用，避免重复创建
                    if let Some(ip) = generate_random_ipv4_address(network, &mut rng) {
                        if ip_tx.send(ip).is_err() {
                            return;
                        }
                        sent_count += 1;
                    }
                }
            }
        }
    }
}

// 流式处理IPv6地址并发送到通道
fn stream_ipv6_to_channel(network: &IpNet, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>, custom_count: Option<usize>) {
    if let IpNet::V6(ipv6_net) = network {
        let prefix = ipv6_net.prefix_len();
        // 使用自定义数量或计算采样数量
        let ip_count = custom_count.unwrap_or_else(|| calculate_sample_count(prefix, false));

        // 创建一个随机数生成器实例
        let mut rng = rand::rng();

        // 直接枚举所有IP再随机采样
        if prefix >= 117 && custom_count.is_none() {
            // 使用 hosts() 方法直接获取所有 IP 地址
            let all_ips: Vec<Ipv6Addr> = ipv6_net.hosts().collect();
            use rand::seq::SliceRandom;
            let sample_count = ip_count.min(all_ips.len());
            let mut sampled = all_ips;
            sampled.shuffle(&mut rng);
            
            // 只保留需要的IP，释放多余内存
            let sampled = sampled.into_iter().take(sample_count).collect::<Vec<_>>();
            
            // 严格按照请求信号发送IP
            for ip in sampled {
                // 等待请求信号
                if req_rx.recv().is_err() {
                    return;
                }
                // 发送IP
                if ip_tx.send(IpAddr::V6(ip)).is_err() {
                    return;
                }
            }
        } else {
            // 其他范围直接随机生成，不去重
            let mut sent_count = 0;
            
            while sent_count < ip_count {
                // 等待请求信号
                if req_rx.recv().is_err() {
                    return;
                }
                // 传递随机数生成器的引用，避免重复创建
                if let Some(ip) = generate_random_ipv6_address(network, &mut rng) {
                    if ip_tx.send(ip).is_err() {
                        return;
                    }
                    sent_count += 1;
                }
            }
        }
    }
}

// 统一的采样数量计算函数
pub fn calculate_sample_count(prefix: u8, is_ipv4: bool) -> usize {
    if is_ipv4 {
        // IPv4 小范围 CIDR 手动控制数量
        match prefix {
            31 => 1,    // /31 只测试 1 个 IP
            30 => 2,    // /30 测试 2 个 IP
            29 => 4,    // /29 测试 4 个 IP
            28 => 8,    // /28 测试 8 个 IP
            27 => 16,   // /27 测试 16 个 IP
            26 => 48,   // /26 测试 48 个 IP
            25 => 96,  // /25 测试 96 个 IP
            24 => 200,  // /24 测试 200 个 IP
            23 => 400,  // /23 测试 400 个 IP
            _ => {
                // 对于更大范围的 CIDR，使用指数函数计算
                let a = 800_000.0;
                let k = 0.35;
                let c = 0.0;
                let result = a * (-k * prefix as f64).exp() + c;
                result.round() as usize
            }
        }
    } else {
        // IPv6 小范围 CIDR 手动控制数量
        match prefix {
            127 => 1,    // /127 只测试 1 个 IP
            126 => 2,    // /126 测试 2 个 IP
            125 => 4,    // /125 测试 4 个 IP
            124 => 8,    // /124 测试 8 个 IP
            123 => 16,   // /123 测试 16 个 IP
            122 => 48,   // /122 测试 48 个 IP
            121 => 96,  // /121 测试 96 个 IP
            120 => 200,  // /120 测试 200 个 IP
            119 => 400,  // /119 测试 400 个 IP
            118 => 800,  // /118 测试 800 个 IP
            117 => 1600, // /117 测试 1600 个 IP
            _ => {
                // 对于更大范围的 CIDR，使用指数函数计算
                let a = 800_000.0;
                let k = 0.05;
                let c = 0.0;
                let result = a * (-k * prefix as f64).exp() + c;
                result.round() as usize
            }
        }
    }
}

// 通用的IPv4地址生成函数
pub fn generate_random_ipv4_address(ip_net: &IpNet, rng: &mut impl Rng) -> Option<IpAddr> {
    match ip_net {
        IpNet::V4(ipv4_net) => {
            let network_addr = ipv4_net.network();
            let broadcast_addr = ipv4_net.broadcast();
            
            // 将IP地址转换为u32
            let start = u32::from_be_bytes(network_addr.octets());
            let end = u32::from_be_bytes(broadcast_addr.octets());
            
            // 在范围内生成随机数
            let random_ip =rng.random_range(start..=end);
            
            // 转回IPv4地址
            let ip = Ipv4Addr::from(random_ip.to_be_bytes());
            Some(IpAddr::V4(ip))
        }
        _ => None,
    }
}

// 通用的IPv6地址生成函数
pub fn generate_random_ipv6_address(ip_net: &IpNet, rng: &mut impl Rng) -> Option<IpAddr> {
    match ip_net {
        IpNet::V6(ipv6_net) => {
            let network_addr = ipv6_net.network();
            let broadcast_addr = ipv6_net.broadcast();
            
            // 将IPv6地址转换为u128
            let start = u128::from_be_bytes(network_addr.octets());
            let end = u128::from_be_bytes(broadcast_addr.octets());
            
            // 在范围内生成随机数
            let random_ip =rng.random_range(start..=end);
            
            // 转回IPv6地址
            let ip = Ipv6Addr::from(random_ip.to_be_bytes());
            Some(IpAddr::V6(ip))
        }
        _ => None,
    }
}

// 读取文件行
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())}
