use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::str::FromStr;
use rand::Rng;
use ipnetwork::IpNetwork;
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
    
    // 收集IP源
    let mut ip_sources = Vec::new();
    
    if !ip_text.is_empty() {
        // 从参数中获取IP段数据
        let ips: Vec<&str> = ip_text.split(',').collect();
        for ip in ips {
            let ip = ip.trim();
            if !ip.is_empty() {
                ip_sources.push(ip.to_string());
            }
        }
    } else if !ip_url.is_empty() {
        // 从URL获取IP段数据
        match fetch_ip_from_url(&ip_url) {
            Ok(content) => {
                // 按行处理获取的内容
                for line in content.lines() {
                    let line = line.trim();
                    if !line.is_empty() {
                        ip_sources.push(line.to_string());
                    }
                }
            },
            Err(err) => {
                println!("从URL获取IP段数据失败: {}", err);
            }
        }
    } else {
        // 从文件中获取IP段数据
        if let Ok(lines) = read_lines(&ip_file) {
            for line in lines.flatten() {
                let line = line.trim();
                if !line.is_empty() {
                    ip_sources.push(line.to_string());
                }
            }
        } else {
            println!("无法读取IP文件: {}", ip_file);
        }
    }
    
    // 先计算总IP数量
    let mut total_expected = 0;
    
    for ip_range in &ip_sources {
        // 先尝试解析为单个IP
        if IpAddr::from_str(ip_range).is_ok() {
            total_expected += 1;
            continue;
        }
        
        // 如果不是单个IP，再尝试解析为CIDR
        if let Ok(network) = IpNetwork::from_str(ip_range) {
            if is_ipv4(ip_range) {
                if let IpNetwork::V4(ipv4_net) = network {
                    if test_all {
                        let prefix = ipv4_net.prefix();
                        let total_ips = if prefix < 32 { 2u32.pow((32 - prefix) as u32) as usize } else { 1 };
                        total_expected += total_ips;
                    } else {
                        let prefix = ipv4_net.prefix();
                        let sample_count = calculate_sample_count(prefix, true);
                        total_expected += sample_count;
                    }
                }
            } else {
                if let IpNetwork::V6(ipv6_net) = network {
                    let prefix = ipv6_net.prefix();
                    let sample_count = calculate_sample_count(prefix, false);
                    total_expected += sample_count;
                }
            }
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

// 从URL获取IP段数据
fn fetch_ip_from_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 创建单线程运行时
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()?;
            
            // 重试逻辑
            let max_retries = 3;
            let mut retry_count = 0;
            let mut last_error = None;
            
            while retry_count < max_retries {
                match client.get(url)
                    .header("User-Agent", USER_AGENT)
                    .send()
                    .await {
                        Ok(response) => {
                            // 检查状态码
                            if !response.status().is_success() {
                                retry_count += 1;
                                last_error = Some(format!("HTTP请求失败，状态码: {}", response.status()));
                                println!("请求失败，状态码: {}，正在重试 ({}/{})", response.status(), retry_count, max_retries);
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                continue;
                            }
                            
                            // 获取响应内容
                            match response.text().await {
                                Ok(content) => return Ok(content),
                                Err(e) => {
                                    retry_count += 1;
                                    last_error = Some(format!("读取响应内容失败: {}", e));
                                    println!("读取响应内容失败: {}，正在重试 ({}/{})", e, retry_count, max_retries);
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                }
                            }
                        },
                        Err(e) => {
                            retry_count += 1;
                            last_error = Some(format!("发送HTTP请求失败: {}", e));
                            println!("发送HTTP请求失败: {}，正在重试 ({}/{})", e, retry_count, max_retries);
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    }
            }
            
            Err(last_error.unwrap_or_else(|| "未知错误".to_string()).into())
        })
}

// 处理IP范围并发送到通道
fn process_ip_range_to_channel(ip_range: &str, test_all: bool, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>) {
    // 忽略注释行
    if ip_range.starts_with('#') || ip_range.starts_with("//") {
        return;
    }
    
    // 尝试直接解析为单个IP地址
    if !ip_range.contains('/') {
        if let Ok(ip) = IpAddr::from_str(ip_range) {
            let _ = ip_tx.send(ip);
            return;
        }
        return;
    }
    
    // 处理CIDR格式的IP段
    if let Ok(network) = IpNetwork::from_str(ip_range) {
        // 直接处理单IP的CIDR格式（/32或/128）
        match network {
            IpNetwork::V4(ipv4_net) if ipv4_net.prefix() == 32 => {
                let _ = ip_tx.send(IpAddr::V4(ipv4_net.ip()));
                return;
            },
            IpNetwork::V6(ipv6_net) if ipv6_net.prefix() == 128 => {
                let _ = ip_tx.send(IpAddr::V6(ipv6_net.ip()));
                return;
            },
            _ => {
                // 处理其他CIDR格式
                if is_ipv4(ip_range) {
                    stream_ipv4_to_channel(&network, test_all, ip_tx, req_rx);
                } else {
                    stream_ipv6_to_channel(&network, ip_tx, req_rx);
                }
            }
        }
    }
}

// 判断是否为IPv4
fn is_ipv4(ip: &str) -> bool {
    ip.contains('.')
}
// 流式处理IPv4地址并发送到通道
fn stream_ipv4_to_channel(network: &IpNetwork, test_all: bool, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>) {
    if let IpNetwork::V4(ipv4_net) = network {
        if test_all {
            // 测试所有IP时，严格按照请求信号发送
            let mut iter = ipv4_net.iter();
            while let Some(ip) = iter.next() {
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
            let prefix = ipv4_net.prefix();
            let ip_count = calculate_sample_count(prefix, true);

            // 小于等于/23，直接枚举所有IP再随机采样
            if prefix <= 23 {
                // 先准备好采样的IP列表
                let all_ips: Vec<Ipv4Addr> = ipv4_net.iter().collect();
                let mut rng = rand::rng();
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
                // 创建一个随机数生成器实例，在整个循环中重用
                let mut rng = rand::rng();
                
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
fn stream_ipv6_to_channel(network: &IpNetwork, ip_tx: &Sender<IpAddr>, req_rx: &Receiver<()>) {
    if let IpNetwork::V6(ipv6_net) = network {
        let prefix = ipv6_net.prefix();
        let ip_count = calculate_sample_count(prefix, false);

        // 小于等于/117，直接枚举所有IP再随机采样
        if prefix <= 117 {
            // 先准备好采样的IP列表
            let all_ips: Vec<Ipv6Addr> = ipv6_net.iter().collect();
            let mut rng = rand::rng();
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
            // 创建一个随机数生成器实例，在整个循环中重用
            let mut rng = rand::rng();
            
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
pub fn generate_random_ipv4_address(ip_net: &IpNetwork, rng: &mut impl Rng) -> Option<IpAddr> {
    match ip_net {
        IpNetwork::V4(ipv4_net) => {
            let network_addr = ipv4_net.network();
            let broadcast_addr = ipv4_net.broadcast();

            let network_octets = network_addr.octets();
            let broadcast_octets = broadcast_addr.octets();

            // 初始化结果段为网络地址的段
            let mut result_octets = network_octets;

            for i in 0..4 {
                if network_octets[i] != broadcast_octets[i] {
                    // 该段有范围限制，进行随机
                    result_octets[i] = rng.random_range(network_octets[i]..=broadcast_octets[i]);
                }
                // 如果段相同，保持不变（已在初始化时完成）
            }

            Some(IpAddr::V4(Ipv4Addr::from(result_octets)))
        }
        _ => None,
    }
}

// 通用的IPv6地址生成函数
pub fn generate_random_ipv6_address(ip_net: &IpNetwork, rng: &mut impl Rng) -> Option<IpAddr> {
    match ip_net {
        IpNetwork::V6(ipv6_net) => {
            let network_addr = ipv6_net.network();
            let broadcast_addr = ipv6_net.broadcast();

            let network_octets = network_addr.octets();
            let broadcast_octets = broadcast_addr.octets();

            // 初始化结果段为网络地址的段
            let mut result_octets = network_octets;

            for i in 0..8 {
                if network_octets[i] != broadcast_octets[i] {
                    // 该段有范围限制，进行随机
                    result_octets[i] = rng.random_range(network_octets[i]..=broadcast_octets[i]);
                }
                // 如果段相同，保持不变（已在初始化时完成）
            }

            Some(IpAddr::V6(Ipv6Addr::from(result_octets)))
        }
        _ => None,
    }
}

// 读取文件行
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())}
