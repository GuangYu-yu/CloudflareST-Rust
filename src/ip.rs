use std::fs::File;
use std::io::{self, BufRead, Write, Seek, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::str::FromStr;
use rand::Rng;
use ipnetwork::IpNetwork;
use std::collections::HashSet;
use reqwest;
use std::env;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::sync::mpsc;

use crate::args::Args;
use crate::common::USER_AGENT;  // 导入常量

// IP缓存文件管理结构体
pub struct IpBuffer {
    cache_file: String,
    total_expected: usize,  // 预计总IP数量
    current_position: u64,  // 当前读取位置
    file: Option<File>,     // 读取文件句柄
}

impl IpBuffer {
    pub fn new() -> Self {
        // 创建临时目录路径
        let temp_dir = env::temp_dir();
        let cache_file = temp_dir.join("cloudflarest_ip_cache.bin").to_string_lossy().to_string();
        
        // 确保文件不存在（如果存在则删除）
        if Path::new(&cache_file).exists() {
            let _ = fs::remove_file(&cache_file);
        }
        
        Self {
            cache_file,
            total_expected: 0,
            current_position: 0,
            file: None,
        }
    }

    // 添加IP到缓存文件
    pub fn push(&mut self, ip: IpAddr) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .append(true)
            .create(true)
            .open(&self.cache_file)?;
        
        // 将IP序列化为字节并写入文件
        match ip {
            IpAddr::V4(ipv4) => {
                file.write_all(&[4])?; // 标记为IPv4
                file.write_all(&ipv4.octets())?;
            },
            IpAddr::V6(ipv6) => {
                file.write_all(&[6])?; // 标记为IPv6
                file.write_all(&ipv6.octets())?;
            }
        }
        
        Ok(())
    }

    // 从缓存文件获取下一个IP
    pub fn pop(&mut self) -> Option<IpAddr> {
        // 如果文件句柄不存在，则打开文件
        if self.file.is_none() {
            match File::open(&self.cache_file) {
                Ok(file) => self.file = Some(file),
                Err(_) => return None,
            }
        }
        
        let file = self.file.as_mut().unwrap();
        
        // 设置文件读取位置
        if let Err(_) = file.seek(io::SeekFrom::Start(self.current_position)) {
            return None;
        }
        
        // 读取IP类型标记
        let mut ip_type = [0u8; 1];
        if let Err(_) = file.read_exact(&mut ip_type) {
            return None;
        }
        
        let ip = match ip_type[0] {
            4 => {
                // 读取IPv4地址
                let mut octets = [0u8; 4];
                if let Err(_) = file.read_exact(&mut octets) {
                    return None;
                }
                self.current_position += 5; // 1字节类型 + 4字节IPv4
                Some(IpAddr::V4(Ipv4Addr::from(octets)))
            },
            6 => {
                // 读取IPv6地址
                let mut octets = [0u8; 16];
                if let Err(_) = file.read_exact(&mut octets) {
                    return None;
                }
                self.current_position += 17; // 1字节类型 + 16字节IPv6
                Some(IpAddr::V6(Ipv6Addr::from(octets)))
            },
            _ => None,
        };
        
        ip
    }

    // 获取预计总IP数量
    pub fn total_expected(&self) -> usize {
        self.total_expected
    }

    // 增加预计总IP数量
    pub fn add_to_total_expected(&mut self, count: usize) {
        self.total_expected += count;
    }

    // 判断是否已读取完所有IP
    pub fn is_empty(&self) -> bool {
        if !Path::new(&self.cache_file).exists() {
            return true;
        }
        
        if let Ok(metadata) = fs::metadata(&self.cache_file) {
            return metadata.len() <= self.current_position;
        }
        
        true
    }
    
    // 清理缓存文件
    pub fn cleanup(&self) {
        if Path::new(&self.cache_file).exists() {
            let _ = fs::remove_file(&self.cache_file);
        }
    }
}

impl Drop for IpBuffer {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// 加载IP列表到缓存
pub fn load_ip_to_buffer(config: &Args) -> IpBuffer {
    let mut ip_buffer = IpBuffer::new();
    
    if !config.ip_text.is_empty() {
        // 从参数中获取IP段数据
        let ips: Vec<&str> = config.ip_text.split(',').collect();
        for ip in ips {
            let ip = ip.trim();
            if ip.is_empty() {
                continue;
            }
            
            process_ip_range_to_cache(ip, config.test_all, &mut ip_buffer);
        }
    } else if !config.ip_url.is_empty() {
        // 从URL获取IP段数据
        match fetch_ip_from_url(&config.ip_url) {
            Ok(content) => {
                // 按行处理获取的内容
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    
                    process_ip_range_to_cache(line, config.test_all, &mut ip_buffer);
                }
            },
            Err(err) => {
                println!("从URL获取IP段数据失败: {}", err);
            }
        }
    } else {
        // 从文件中获取IP段数据
        let ip_file = &config.ip_file;
        
        if let Ok(lines) = read_lines(ip_file) {
            for line in lines.flatten() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                
                process_ip_range_to_cache(line, config.test_all, &mut ip_buffer);
            }
        } else {
            println!("无法读取IP文件: {}", ip_file);
        }
    }
    ip_buffer
}

// 从URL获取IP段数据
fn fetch_ip_from_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 创建一个阻塞的HTTP客户端，设置超时
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))  // 设置5秒超时
        .build()?;
    
    // 重试逻辑
    let max_retries = 3;
    let mut retry_count = 0;
    let mut last_error = None;
    
    while retry_count < max_retries {
        match client.get(url)
            .header("User-Agent", USER_AGENT)  // 使用导入的常量
            .send() {
                Ok(response) => {
                    // 检查状态码
                    if !response.status().is_success() {
                        retry_count += 1;
                        last_error = Some(format!("HTTP请求失败，状态码: {}", response.status()));
                        println!("请求失败，状态码: {}，正在重试 ({}/{})", response.status(), retry_count, max_retries);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                    
                    // 获取响应内容
                    match response.text() {
                        Ok(content) => return Ok(content),
                        Err(e) => {
                            retry_count += 1;
                            last_error = Some(format!("读取响应内容失败: {}", e));
                            println!("读取响应内容失败: {}，正在重试 ({}/{})", e, retry_count, max_retries);
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                },
                Err(e) => {
                    retry_count += 1;
                    last_error = Some(format!("发送HTTP请求失败: {}", e));
                    println!("发送HTTP请求失败: {}，正在重试 ({}/{})", e, retry_count, max_retries);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
    }
    
    // 所有重试都失败了
    Err(last_error.unwrap_or_else(|| "未知错误".to_string()).into())
}

// 处理IP范围并添加到缓存
fn process_ip_range_to_cache(ip_range: &str, test_all: bool, ip_cache: &mut IpBuffer) {
    // 忽略注释行
    if ip_range.starts_with('#') || ip_range.starts_with("//") {
        return;
    }
    
    // 尝试直接解析为单个IP地址
    if !ip_range.contains('/') {
        if let Ok(ip) = IpAddr::from_str(ip_range) {
            let _ = ip_cache.push(ip);
            ip_cache.add_to_total_expected(1);
        }
        return;
    }
    
    // 处理CIDR格式的IP段
    if let Ok(network) = IpNetwork::from_str(ip_range) {
        // 直接处理单IP的CIDR格式（/32或/128）
        match network {
            IpNetwork::V4(ipv4_net) if ipv4_net.prefix() == 32 => {
                let _ = ip_cache.push(IpAddr::V4(ipv4_net.ip()));
                ip_cache.add_to_total_expected(1);
                return;
            },
            IpNetwork::V6(ipv6_net) if ipv6_net.prefix() == 128 => {
                let _ = ip_cache.push(IpAddr::V6(ipv6_net.ip()));
                ip_cache.add_to_total_expected(1);
                return;
            },
            _ => {
                // 处理其他CIDR格式
                if is_ipv4(ip_range) {
                    choose_ipv4_to_cache(&network, test_all, ip_cache);
                } else {
                    choose_ipv6_to_cache(&network, ip_cache);
                }
            }
        }
    }
}

// 判断是否为IPv4
fn is_ipv4(ip: &str) -> bool {
    ip.contains('.')
}

// 选择IPv4地址并添加到缓存
fn choose_ipv4_to_cache(network: &IpNetwork, test_all: bool, ip_cache: &mut IpBuffer) {
    if let IpNetwork::V4(ipv4_net) = network {
        if test_all {
            // 测试所有IP
            let total_ips = 2u32.pow((32 - ipv4_net.prefix()) as u32) as usize;
            ip_cache.add_to_total_expected(total_ips);
            
            for ip in ipv4_net.iter() {
                let _ = ip_cache.push(IpAddr::V4(ip));
            }
        } else {
            // 根据网络大小生成不同数量的随机IP
            let prefix = ipv4_net.prefix();
            
            // 使用函数计算IP数量
            let ip_count = calculate_sample_count(prefix, true);
            ip_cache.add_to_total_expected(ip_count);

            // 使用多线程生成IP
            if ip_count > 10000 {
                generate_ips_with_threads(network, ip_count, ip_cache);
            } else {
                // 对于小数量的IP，使用单线程处理
                let mut generated_ips = HashSet::new();
                while generated_ips.len() < ip_count {
                    if let Some(ip_str) = generate_random_ipv4_address(network) {
                        if let Ok(ip) = Ipv4Addr::from_str(&ip_str) {
                            if generated_ips.insert(ip) { // 如果插入成功，说明是新IP
                                let _ = ip_cache.push(IpAddr::V4(ip));
                            }
                        }
                    }
                }
            }
        }
    }
}

// 选择IPv6地址并添加到缓存
fn choose_ipv6_to_cache(network: &IpNetwork, ip_cache: &mut IpBuffer) {
    if let IpNetwork::V6(ipv6_net) = network {
        let prefix = ipv6_net.prefix();
        
        // 使用函数计算IP数量
        let ip_count = calculate_sample_count(prefix, false);
        ip_cache.add_to_total_expected(ip_count);

        // 使用多线程生成IP
        if ip_count > 10000 {
            generate_ips_with_threads(network, ip_count, ip_cache);
        } else {
            // 对于小数量的IP，使用单线程处理
            let mut generated_ips = HashSet::new();
            while generated_ips.len() < ip_count {
                if let Some(ip_str) = generate_random_ipv6_address(network) {
                    if let Ok(ip) = Ipv6Addr::from_str(&ip_str) {
                        if generated_ips.insert(ip) { // 如果插入成功，说明是新IP
                            let _ = ip_cache.push(IpAddr::V6(ip));
                        }
                    }
                }
            }
        }
    }
}

// 使用多线程生成IP地址
fn generate_ips_with_threads(network: &IpNetwork, ip_count: usize, ip_cache: &mut IpBuffer) {
    
    // 创建通道用于收集生成的IP
    let (tx, rx) = mpsc::channel();
    
    // 计算每个线程需要生成的IP数量
    let thread_count = 32; // 使用32个线程
    let ips_per_thread = (ip_count + thread_count - 1) / thread_count; // 向上取整
    
    // 创建线程池
    let mut handles = vec![];
    
    // 启动多个线程生成IP
    for _ in 0..thread_count {
        let tx = tx.clone();
        let network_clone = network.clone();
        
        let handle = thread::spawn(move || {
            let mut local_ips = HashSet::new();
            let mut count = 0;
            
            // 每个线程生成指定数量的IP
            while count < ips_per_thread {
                let ip_opt = match network_clone {
                    IpNetwork::V4(_) => {
                        if let Some(ip_str) = generate_random_ipv4_address(&network_clone) {
                            if let Ok(ip) = Ipv4Addr::from_str(&ip_str) {
                                Some(IpAddr::V4(ip))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    },
                    IpNetwork::V6(_) => {
                        if let Some(ip_str) = generate_random_ipv6_address(&network_clone) {
                            if let Ok(ip) = Ipv6Addr::from_str(&ip_str) {
                                Some(IpAddr::V6(ip))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                };
                
                if let Some(ip) = ip_opt {
                    if local_ips.insert(ip) {
                        // 发送IP到主线程
                        let _ = tx.send(ip);
                        count += 1;
                    }
                }
            }
        });
        
        handles.push(handle);
    }
    
    // 丢弃发送端的最后一个副本，这样接收端可以在所有线程完成后退出
    drop(tx);
    
    // 创建一个线程安全的HashSet来跟踪已添加的IP
    let added_ips = Arc::new(Mutex::new(HashSet::new()));
    let target_count = ip_count.min(thread_count * ips_per_thread);
    
    // 从通道接收IP并写入缓存文件
    let mut received_count = 0;
    for ip in rx {
        let mut added_ips_guard = added_ips.lock().unwrap();
        if added_ips_guard.insert(ip) {
            let _ = ip_cache.push(ip);
            received_count += 1;
            
            // 如果已经收集到足够的IP，就退出
            if received_count >= target_count {
                break;
            }
        }
    }
    
    // 等待所有线程完成
    for handle in handles {
        let _ = handle.join();
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
            _ => {
                // 对于更大范围的 CIDR，使用指数函数计算
                let a = 800_000.0;
                let k = 0.1;
                let c = 0.0;
                let result = a * (-k * prefix as f64).exp() + c;
                result.round() as usize
            }
        }
    }
}

// 通用的IPv4地址生成函数
pub fn generate_random_ipv4_address(ip_net: &IpNetwork) -> Option<String> {
    match ip_net {
        IpNetwork::V4(ipv4_net) => {
            // 获取网络地址和掩码
            let ip = ipv4_net.network().octets();
            let ones = ipv4_net.prefix();
            let random_bits = 32 - ones;

            // 将IP地址转换为u32
            let base_ip = u32::from_be_bytes(ip);

            // 创建掩码
            let net_mask = 0xffffffff << random_bits;
            let network_addr = base_ip & net_mask;

            // 计算最大偏移量
            let max_offset = 1u32 << random_bits;

            if max_offset == 1 {
                // /32，只有一个IP，直接返回
                return Some(ipv4_net.network().to_string());
            }

            // 生成随机偏移量
            let random_offset = if max_offset > 2 {
                rand::rng().random_range(1..max_offset)
            } else {
                1 // /31 的情况，两个地址都可以用
            };

            // 计算最终IP
            let final_ip = network_addr | random_offset;

            // 转换回IP地址格式
            let result = Ipv4Addr::from(final_ip.to_be_bytes());
            Some(result.to_string())
        }
        _ => None,
    }
}

// 通用的IPv6地址生成函数
pub fn generate_random_ipv6_address(ip_net: &IpNetwork) -> Option<String> {
    match ip_net {
        IpNetwork::V6(ipv6_net) => {
            // 获取网络地址
            let ip = ipv6_net.network().octets();
            let ones = ipv6_net.prefix();
            let random_bits = 128 - ones;

            // 创建新IP
            let mut new_ip = [0u8; 16];
            new_ip.copy_from_slice(&ip);

            // 计算需要随机的字节数和位数
            let random_bytes = (random_bits / 8) as usize;
            let remaining_bits = random_bits % 8;

            let mut rng = rand::rng();

            // 完全随机的字节
            for i in 0..16 {
                // 只处理需要随机化的字节
                if i >= 16 - random_bytes {
                    // 生成完全随机的字节
                    let rand_value = rng.random::<u8>();
                    // 保留网络前缀部分
                    let mask_byte = if ones > 0 && i == 16 - random_bytes - 1 && remaining_bits > 0 {
                        0xFF << remaining_bits
                    } else if i < (ones as usize) / 8 {
                        0xFF
                    } else {
                        0
                    };
                    new_ip[i] = (new_ip[i] & mask_byte) | (rand_value & !mask_byte);
                }
            }

            // 处理剩余的不足一个字节的位
            if remaining_bits > 0 {
                let byte_pos = 16 - random_bytes - 1;

                // 创建位掩码，只修改需要随机的位
                let bit_mask = 0xFF >> (8 - remaining_bits);
                // 生成随机值
                let rand_value = rng.random::<u8>() & bit_mask;
                // 应用掩码和随机值
                let mask_byte = 0xFF << remaining_bits;
                // 保留网络前缀，修改主机部分
                new_ip[byte_pos] = (new_ip[byte_pos] & mask_byte) | (rand_value & !mask_byte);
            }

            // 检查生成的IP是否为全零地址
            let is_zero = new_ip.iter().all(|&b| b == 0);

            // 如果是全零地址，重新生成
            if is_zero {
                // 简单地将最后一个字节设为1，确保不是全零地址
                new_ip[15] = 1;
            }

            let result = Ipv6Addr::from(new_ip);
            Some(result.to_string())
        }
        _ => None,
    }
}

// 读取文件行
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}
