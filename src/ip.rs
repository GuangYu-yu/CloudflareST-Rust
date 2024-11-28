use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::io;
use ipnet::IpNet;
use rand::Rng;
use crate::types::Config;
use futures::StreamExt;
use num_cpus;
use crate::types::parse_test_amount;
use rand::seq::SliceRandom;

// 全局变量
// pub static mut IP_FILE: &str = "ip.txt";
pub fn init_rand_seed() {
    let mut rng = rand::thread_rng();
    rng.gen::<u64>();
}

fn is_ipv4(ip: &str) -> bool {
    ip.contains('.')
}

fn rand_ip_end_with(num: u8) -> u8 {
    if num == 0 { // 对于 /32 这种单独的 IP
        return 0;
    }
    rand::thread_rng().gen_range(0..=num)
}

#[derive(Clone)]
pub struct IPWithPort {
    pub ip: IpAddr,
    pub port: Option<u16>,
}

impl IPWithPort {
    // 获取实际使用的端口：优先使用指定端口，其次使用配置端口，最后使用默认端口443
    pub fn get_port(&self, config_port: u16) -> u16 {
        self.port.unwrap_or(if config_port != 0 { config_port } else { 443 })
    }
}

struct IPRanges {
    ips: Vec<IPWithPort>,
    mask: String,
    first_ip: Vec<u8>,
    ip_net: IpNet,
    config: Config,
}

impl IPRanges {
    fn new(config: Config) -> Self {
        Self {
            ips: Vec::new(),
            mask: String::new(),
            first_ip: Vec::new(),
            ip_net: "0.0.0.0/0".parse().unwrap(),
            config,
        }
    }

    // 如果是单独 IP 则加上子网掩码，反之则获取子网掩码(r.mask)
    fn fix_ip(&mut self, ip_str: &str) -> (String, Option<u16>) {
        // 处理 IPv6 带端口的情况: [2606:4700::]:443
        if ip_str.contains('[') && ip_str.contains(']') {
            if let (Some(start), Some(end)) = (ip_str.find('['), ip_str.find(']')) {
                let ip = &ip_str[start + 1..end];
                let port = if end + 1 < ip_str.len() && ip_str[end + 1..].starts_with(':') {
                    ip_str[end + 2..].parse::<u16>().ok()
                } else {
                    None
                };
                return (ip.to_string(), port);
            }
        }
        
        // 处理 IPv4 带端口的情况: 1.1.1.1:443
        if ip_str.contains(':') && is_ipv4(ip_str) {
            let parts: Vec<&str> = ip_str.split(':').collect();
            if parts.len() == 2 {
                return (parts[0].to_string(), parts[1].parse::<u16>().ok());
            }
        }

        // 没有指定端口时返回 None
        (ip_str.to_string(), None)
    }

    // 解析 IP 段，获得 IP、IP 范围、子网掩码
    fn parse_cidr(&mut self, ip: &str) {
        let (ip_str, port) = self.fix_ip(ip);
        
        // 如果不含有 '/' 则代表是单个 IP，直接解析这个 IP
        let ip_net = if !ip_str.contains('/') {
            if let Ok(single_ip) = ip_str.parse::<IpAddr>() {
                // 对于单个 IP，直接添加到列表中并返回 None
                self.append_ip(single_ip, port);
                return;
            } else {
                return;  // 解析失败就静默跳过
            }
        } else {
            // 是 CIDR，直接解析
            ip_str.parse()
        };

        // 如果解析失败就静默跳过
        if let Ok(parsed_net) = ip_net {
            self.ip_net = parsed_net;
            self.first_ip = match self.ip_net.addr() {
                IpAddr::V4(ipv4) => ipv4.octets().to_vec(),
                IpAddr::V6(ipv6) => ipv6.octets().to_vec(),
            };
        }
    }

    fn append_ipv4(&mut self, d: u8) {
        let ip = IpAddr::V4(Ipv4Addr::new(
            self.first_ip[0],
            self.first_ip[1], 
            self.first_ip[2],
            d
        ));
        self.ips.push(IPWithPort { ip, port: None });
    }

    fn append_ip(&mut self, ip: IpAddr, port: Option<u16>) {
        self.ips.push(IPWithPort { ip, port });
    }

    // 返回第四段 ip 的最小值及可用数目
    fn get_ip_range(&self) -> (u8, u8) {
        let min_ip = match &self.ip_net {
            IpNet::V4(ipv4_net) => {
                // 对于 IPv4，获取掩码并计算最小 IP
                let mask = ipv4_net.netmask().octets();
                self.first_ip[3] & mask[3]
            }
            IpNet::V6(_) => {
                // 对于 IPv6，直接返回当前值
                self.first_ip[3]
            }
        };

        // 计算可用主机数
        let hosts = match self.ip_net.prefix_len() {
            32 => 0,  // 单个 IP
            24 => 255,  // 整个 /24 网段
            n if n < 32 => {
                // 计算主机位数量
                let host_bits = 32 - n;
                let total = (1u32 << host_bits) - 1;
                // 如果超过 255，则限制为 255
                if total > 255 {
                    255
                } else {
                    total as u8
                }
            }
            _ => 0,  // 其他情况（包括 IPv6）
        };

        (min_ip, hosts)
    }

    fn choose_ips(&mut self, is_ipv4: bool, config: &Config) {
        let test_amount = if is_ipv4 {
            match config.ipv4_amount {
                Some(amount) => parse_test_amount(&amount.to_string(), true),
                None => if config.test_all { 2u32.pow(16) } else { 1 }
            }
        } else {
            match config.ipv6_amount {
                Some(amount) => parse_test_amount(&amount.to_string(), false),
                None => if config.test_all { 2u32.pow(20) } else { 1 }
            }
        };

        // 如果没有指定任何特殊的 IP 测试数量参数，使用原始的随机逻辑
        if is_ipv4 && config.ipv4_amount.is_none() && !config.test_all {
            self.choose_ipv4_original(config.test_all);
            return;
        }
        if !is_ipv4 && config.ipv6_amount.is_none() {
            self.choose_ipv6_original();
            return;
        }

        // 否则使用新的测试数量逻辑
        let cidr_size = if is_ipv4 {
            match self.mask.trim_start_matches('/').parse::<u32>() {
                Ok(n) => 2u32.pow(32 - n),
                Err(_) => return
            }
        } else {
            match self.mask.trim_start_matches('/').parse::<u32>() {
                Ok(n) => 2u32.pow(128 - n),
                Err(_) => return
            }
        };

        // 如果测试数量大于等于 CIDR 大小，测试全部
        if test_amount >= cidr_size {
            if is_ipv4 {
                self.choose_ipv4_original(true);  // 测试所有 IP
            } else {
                self.choose_ipv6_original();  // IPv6 保持原有逻辑
            }
            return;
        }

        // 否则根据 IP 类型选择对应的随机选择逻辑
        if is_ipv4 {
            self.choose_ipv4_original(false);
        } else {
            // 使用改进的 IPv6 生成方法替代原始方法
            self.choose_ipv6_improved();
        }
    }

    // 原始的 IPv4 随机选择逻辑
    fn choose_ipv4_original(&mut self, test_all: bool) {
        if self.mask == "/32" {
            // 单个 IP 则无需随机，直接加入自身即可
            self.append_ip(IpAddr::V4(Ipv4Addr::new(
                self.first_ip[0],
                self.first_ip[1],
                self.first_ip[2],
                self.first_ip[3],
            )), None);
            return;
        }

        let (min_ip, hosts) = self.get_ip_range();
        
        while self.ip_net.contains(&IpAddr::V4(Ipv4Addr::new(
            self.first_ip[0],
            self.first_ip[1],
            self.first_ip[2],
            self.first_ip[3],
        ))) {
            if test_all {
                // 如果是测速全部 IP
                for i in 0..=hosts {
                    self.append_ipv4(i + min_ip);
                }
            } else {
                // 随机 IP 的最后一段 0.0.0.X
                self.append_ipv4(min_ip + rand_ip_end_with(hosts));
            }

            self.first_ip[2] += 1; // 0.0.(X+1).X
            if self.first_ip[2] == 0 {
                self.first_ip[1] += 1; // 0.(X+1).X.X
                if self.first_ip[1] == 0 {
                    self.first_ip[0] += 1; // (X+1).X.X.X
                }
            }
        }
    }

    // 原始的 IPv6 随机选择逻辑
    fn choose_ipv6_original(&mut self) {
        if self.mask == "/128" {
            // 单个 IP 则无需随机，直接加入自身即可
            let mut ip_bytes = [0u16; 8];
            for i in 0..8 {
                ip_bytes[i] = ((self.first_ip[i*2] as u16) << 8) | (self.first_ip[i*2+1] as u16);
            }
            self.append_ip(IpAddr::V6(Ipv6Addr::new(
                ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3],
                ip_bytes[4], ip_bytes[5], ip_bytes[6], ip_bytes[7],
            )), None);
            return;
        }

        while self.ip_net.contains(&IpAddr::V6(Ipv6Addr::new(
            ((self.first_ip[0] as u16) << 8) | (self.first_ip[1] as u16),
            ((self.first_ip[2] as u16) << 8) | (self.first_ip[3] as u16),
            ((self.first_ip[4] as u16) << 8) | (self.first_ip[5] as u16),
            ((self.first_ip[6] as u16) << 8) | (self.first_ip[7] as u16),
            ((self.first_ip[8] as u16) << 8) | (self.first_ip[9] as u16),
            ((self.first_ip[10] as u16) << 8) | (self.first_ip[11] as u16),
            ((self.first_ip[12] as u16) << 8) | (self.first_ip[13] as u16),
            ((self.first_ip[14] as u16) << 8) | (self.first_ip[15] as u16),
        ))) {
            self.first_ip[15] = rand_ip_end_with(255);
            self.first_ip[14] = rand_ip_end_with(255);

            let target_ip = self.first_ip.clone();
            self.append_ip(IpAddr::V6(Ipv6Addr::new(
                ((target_ip[0] as u16) << 8) | (target_ip[1] as u16),
                ((target_ip[2] as u16) << 8) | (target_ip[3] as u16),
                ((target_ip[4] as u16) << 8) | (target_ip[5] as u16),
                ((target_ip[6] as u16) << 8) | (target_ip[7] as u16),
                ((target_ip[8] as u16) << 8) | (target_ip[9] as u16),
                ((target_ip[10] as u16) << 8) | (target_ip[11] as u16),
                ((target_ip[12] as u16) << 8) | (target_ip[13] as u16),
                ((target_ip[14] as u16) << 8) | (target_ip[15] as u16),
            )), None);

            // 更新 IP
            for i in (0..14).rev() {
                let temp_ip = self.first_ip[i];
                self.first_ip[i] = self.first_ip[i].wrapping_add(rand_ip_end_with(255));
                if self.first_ip[i] >= temp_ip {
                    break;
                }
            }
        }
    }

    pub async fn load_ips_concurrent(&mut self, ip_text: &str, test_all: bool, config: &Config) -> io::Result<()> {
        let ip_chunks: Vec<_> = ip_text
            .lines()  // 按行分割
            .filter_map(|line| {
                // 移除注释
                let line = if let Some(idx) = line.find('#') {
                    &line[..idx]
                } else if let Some(idx) = line.find("//") {
                    &line[..idx]
                } else {
                    line
                };
                
                // 去除空白字符并检查是否为空
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                
                // 处理逗号分隔的多个IP/CIDR
                Some(line.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>())
            })
            .flatten()
            .collect();
            
        let mut handles = Vec::new();
        
        for chunk in ip_chunks {
            let chunk = chunk.to_string();
            let _test_all = test_all;
            let config = config.clone();
            let handle = tokio::spawn(async move {
                let mut ranges = IPRanges::new(config.clone());
                ranges.parse_cidr(&chunk);  // 解析失败会静默跳过
                if is_ipv4(&chunk) {
                    ranges.choose_ips(true, &config);
                } else {
                    ranges.choose_ips(false, &config);
                }
                ranges.ips
            });
            handles.push(handle);
        }

        // 使用 stream 并发收集结果
        let results = futures::stream::iter(handles)
            .buffer_unordered(num_cpus::get()) // 使用 CPU 核心数作为并发数
            .collect::<Vec<_>>()
            .await;
            
        // 合并结果
        for result in results {
            if let Ok(ips) = result {
                self.ips.extend(ips);
            }
        }
        
        // 在合并结果后添加随机采样
        if self.ips.len() > 500000 {
            self.random_sample(500000);
        }
        
        Ok(())
    }

    // 添加随机采样方法
    fn random_sample(&mut self, sample_size: usize) {
        let mut rng = rand::thread_rng();
        
        // 使用 SliceRandom trait 的 shuffle 方法
        self.ips[..sample_size].shuffle(&mut rng);
        
        // 截取前 sample_size 个元素
        self.ips.truncate(sample_size);
    }

    fn choose_ipv6_improved(&mut self) {
        if self.mask == "/128" {
            // 单个 IP 直接添加
            self.append_ipv6_single();
            return;
        }

        let prefix_len = self.mask.trim_start_matches('/').parse::<u32>().unwrap_or(128);
        let host_bits = 128 - prefix_len;
        
        // 使用 Fisher-Yates 算法生成不重复的随机序列
        let mut rng = rand::thread_rng();
        let amount = match &self.config.ipv6_num_mode {
            Some(mode) => match mode.as_str() {
                "more" => 262144, // 2^18
                "lots" => 65536,  // 2^16
                "many" => 4096,   // 2^12
                "some" => 256,    // 2^8
                _ => 1
            },
            None => 1
        };

        let mut indices: Vec<u32> = (0..2u32.pow(host_bits.min(32)))
            .take(amount as usize)
            .collect();
        
        // 只打乱我们需要的数量
        for i in 0..amount.min(indices.len() as u32) {
            let j = rng.gen_range(i..indices.len() as u32);
            indices.swap(i as usize, j as usize);
        }

        // 生成 IP
        for i in 0..amount.min(indices.len() as u32) {
            let mut ip_bytes = self.first_ip.clone();
            let index = indices[i as usize];
            
            // 从高位到低位依次填充主机部分
            let mut remaining = index;
            for j in ((16 - (host_bits + 7) / 8) as usize)..16 {
                ip_bytes[j] = (remaining & 0xFF) as u8;
                remaining >>= 8;
            }

            // 验证并添加 IP
            let ip = IpAddr::V6(self.bytes_to_ipv6(&ip_bytes));
            if self.ip_net.contains(&ip) {
                self.append_ip(ip, None);
            }
        }
    }

    fn append_ipv6_single(&mut self) {
        let mut ip_bytes = [0u16; 8];
        for i in 0..8 {
            ip_bytes[i] = ((self.first_ip[i*2] as u16) << 8) | (self.first_ip[i*2+1] as u16);
        }
        self.append_ip(IpAddr::V6(Ipv6Addr::new(
            ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3],
            ip_bytes[4], ip_bytes[5], ip_bytes[6], ip_bytes[7],
        )), None);
    }

    fn bytes_to_ipv6(&self, bytes: &[u8]) -> Ipv6Addr {
        Ipv6Addr::new(
            ((bytes[0] as u16) << 8) | (bytes[1] as u16),
            ((bytes[2] as u16) << 8) | (bytes[3] as u16),
            ((bytes[4] as u16) << 8) | (bytes[5] as u16),
            ((bytes[6] as u16) << 8) | (bytes[7] as u16),
            ((bytes[8] as u16) << 8) | (bytes[9] as u16),
            ((bytes[10] as u16) << 8) | (bytes[11] as u16),
            ((bytes[12] as u16) << 8) | (bytes[13] as u16),
            ((bytes[14] as u16) << 8) | (bytes[15] as u16),
        )
    }

    // 添加一个新函数来获取 IpAddr 列表
    pub fn get_ips(&self) -> Vec<IpAddr> {
        self.ips.iter().map(|ip_with_port| ip_with_port.ip).collect()
    }
}

pub async fn load_ip_ranges_concurrent(config: &Config) -> io::Result<Vec<IpAddr>> {
    let mut ranges = IPRanges::new(config.clone());
    let test_all = config.is_test_all();
    
    if !config.ip_text.is_empty() {
        ranges.load_ips_concurrent(&config.ip_text, test_all, &config).await?;
    } else {
        let file_path = if config.ip_file.is_empty() { "ip.txt" } else { &config.ip_file };
        if let Ok(content) = std::fs::read_to_string(file_path) {
            ranges.load_ips_concurrent(&content, test_all, &config).await?;
        }
    }
    
    Ok(ranges.get_ips())
}