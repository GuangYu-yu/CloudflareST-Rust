use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::io;
use ipnet::IpNet;
use rand::Rng;
use rand::RngCore;
use crate::types::Config;
use futures::StreamExt;
use num_cpus;
use crate::types::parse_test_amount;
use tracing::debug;
use std::str::FromStr;
use num_bigint::BigUint;
use num_traits::{One, Zero, ToPrimitive};

#[derive(Clone)]
pub struct IPWithPort {
    pub ip: IpAddr,
    pub port: Option<u16>,
}

impl IPWithPort {
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

    fn parse_cidr(&mut self, ip: &str) {
        let (ip_str, port) = self.parse_ip_with_port(ip);
        
        if !ip_str.contains('/') {
            if let Ok(single_ip) = ip_str.parse::<IpAddr>() {
                self.append_ip(single_ip, port);
                return;
            } else {
                return;
            }
        }

        if let Ok(network) = ip_str.parse() {
            self.ip_net = network;
            self.mask = format!("/{}", self.get_prefix_len(&network));
            self.first_ip = self.get_network_bytes(&network);
        }
    }

    fn choose_ipv4(&mut self, test_all: bool, target_amount: Option<u32>) {
        if self.mask == "/32" {
            self.append_ip(self.current_ip(), None);
            return;
        }

        let prefix = self.ip_net.prefix_len();
        let total_ips = BigUint::from(2u32).pow(u32::from(32 - prefix));

        let target = if test_all {
            total_ips.clone()
        } else {
            match target_amount {
                Some(amount) => {
                    let amount = BigUint::from(amount);
                    if amount >= total_ips {
                        // 如果指定数量超过CIDR容量，选取全部IP
                        total_ips.clone()
                    } else {
                        amount
                    }
                },
                None => {
                    // 每64个IP选1个，向上取整
                    (total_ips.clone() + BigUint::from(63u32)) / BigUint::from(64u32)
                }
            }
        };

        if total_ips <= target {
            // 如果需要选取全部IP，使用顺序遍历
            let mut current = BigUint::zero();
            while current < total_ips {
                let ip = self.biguint_to_ipv4(&current);
                if self.ip_net.contains(&ip) {
                    self.append_ip(ip, None);
                }
                current += BigUint::one();
            }
        } else {
            // 使用分层抽样确保均匀分布
            let mut rng = rand::thread_rng();
            let mut selected = Vec::with_capacity(target.to_usize().unwrap_or(0));
            let layer_size = &total_ips / &target + BigUint::one();
            let mut seen = BigUint::zero();
            
            while seen < target {
                let layer_start = &seen * &layer_size;
                let mut layer_end = &layer_start + &layer_size;
                if layer_end > total_ips {
                    layer_end = total_ips.clone();
                }

                // 修改随机数生成逻辑
                let range = &layer_end - &layer_start;
                let mut bytes = vec![0u8; range.to_bytes_be().len()];
                rng.fill_bytes(&mut bytes);
                let mut random_value = BigUint::from_bytes_be(&bytes);
                random_value %= &range;
                let value = layer_start + random_value;
                
                let ip = self.biguint_to_ipv4(&value);
                if self.ip_net.contains(&ip) && !selected.contains(&ip) {
                    selected.push(ip);
                    seen += BigUint::one();
                }
            }
            
            for ip in selected {
                self.append_ip(ip, None);
            }
        }
    }

    fn choose_ipv6(&mut self, target_amount: Option<u32>) {
        if self.mask == "/128" {
            self.append_ip(self.current_ip(), None);
            return;
        }

        let prefix_len = self.ip_net.prefix_len();
        let cidr_capacity = BigUint::from(2u32).pow(u32::from(128 - prefix_len));

        let target = match target_amount {
            Some(amount) => BigUint::from(amount),
            None => {
                match &self.config.ipv6_amount {
                    Some(amount) => BigUint::from(parse_test_amount(&amount.to_string(), false)),
                    None => match self.config.ipv6_num_mode.as_deref() {
                        Some("more") => BigUint::from(2u32).pow(18u32),  // 262144
                        Some("lots") => BigUint::from(2u32).pow(16u32),  // 65536
                        Some("many") => BigUint::from(2u32).pow(12u32),  // 4096
                        Some("some") => BigUint::from(2u32).pow(8u32),   // 256
                        _ => BigUint::from(2u32).pow(8u32)  // 默认256
                    }
                }
            }
        };

        let target = if target > cidr_capacity {
            cidr_capacity.clone()
        } else {
            target
        };
        
        if cidr_capacity <= target {
            self.choose_all_ipv6();
        } else {
            self.choose_random_ipv6(target);
        }
    }

    // 处理IPv6全选逻辑
    fn choose_all_ipv6(&mut self) {
        let fixed_bytes = (self.ip_net.prefix_len() as usize) / 8;
        let remaining_bits = (self.ip_net.prefix_len() as usize) % 8;
        let bytes = self.first_ip.clone();
        let mut value = BigUint::zero();
        let max_value = BigUint::from(2u32).pow(u32::from(128 - self.ip_net.prefix_len()));
        
        while value < max_value {
            let mut ip_bytes = bytes.clone();
            let value_bytes = value.to_bytes_be();
            
            // 处理部分字节
            if remaining_bits > 0 {
                let mask = !((1 << (8 - remaining_bits)) - 1);
                ip_bytes[fixed_bytes] = (bytes[fixed_bytes] & mask) | 
                                      (value_bytes.first().unwrap_or(&0) & !mask);
            }
            
            // 填充剩余字节
            let start = fixed_bytes + (remaining_bits > 0) as usize;
            for i in start..16 {
                let byte_idx = i - start;
                ip_bytes[i] = value_bytes.get(byte_idx).copied().unwrap_or(0);
            }
            
            let ip = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(ip_bytes).unwrap()));
            if self.ip_net.contains(&ip) {
                self.append_ip(ip, None);
            }
            value += BigUint::one();
        }
    }

    // 处理IPv6随机选取逻辑
    fn choose_random_ipv6(&mut self, target_amount: BigUint) {
        let mut rng = rand::thread_rng();
        let mut selected = Vec::with_capacity(target_amount.to_usize().unwrap_or(0));
        let fixed_bytes = (self.ip_net.prefix_len() as usize) / 8;
        let remaining_bits = (self.ip_net.prefix_len() as usize) % 8;
        let max_value = BigUint::from(2u32).pow(u32::from(128 - self.ip_net.prefix_len()));

        // 使用分层抽样策略确保均匀性
        let layer_size = &max_value / &target_amount + BigUint::one();
        let mut seen = BigUint::zero();
        
        while seen < target_amount {
            // 计算当前层的范围
            let layer_start = &seen * &layer_size;
            let mut layer_end = &layer_start + &layer_size;
            if layer_end > max_value {
                layer_end = max_value.clone();
            }

            // 在当前层中随机选择一个值
            let range = &layer_end - &layer_start;
            let mut bytes = vec![0u8; 16];
            rng.fill_bytes(&mut bytes);
            let mut random_value = BigUint::from_bytes_be(&bytes);
            random_value %= &range;
            random_value += &layer_start;

            // 构建 IP
            let mut ip_bytes = [0u8; 16];
            for i in 0..fixed_bytes {
                ip_bytes[i] = self.first_ip[i];
            }
            
            let value_bytes = random_value.to_bytes_be();
            
            if remaining_bits > 0 {
                let mask = !((1 << (8 - remaining_bits)) - 1);
                ip_bytes[fixed_bytes] = (self.first_ip[fixed_bytes] & mask) | 
                                      (value_bytes.first().unwrap_or(&0) & !mask);
            }
            
            let start = fixed_bytes + (remaining_bits > 0) as usize;
            for i in start..16 {
                let byte_idx = i - start;
                ip_bytes[i] = value_bytes.get(byte_idx).copied().unwrap_or(0);
            }
            
            let ip = IpAddr::V6(Ipv6Addr::from(ip_bytes));
            if self.ip_net.contains(&ip) && !selected.contains(&ip) {
                selected.push(ip);
                seen += BigUint::one();
            }
        }

        for ip in selected {
            self.append_ip(ip, None);
        }
    }

    // 其他辅助方法...
    fn get_prefix_len(&self, network: &IpNet) -> u8 {
        network.prefix_len()
    }

    fn get_network_bytes(&self, network: &IpNet) -> Vec<u8> {
        match network {
            IpNet::V4(n) => n.network().octets().to_vec(),
            IpNet::V6(n) => n.network().octets().to_vec(),
        }
    }

    fn current_ip(&self) -> IpAddr {
        match self.ip_net {
            IpNet::V4(_) => {
                IpAddr::V4(Ipv4Addr::new(
                    self.first_ip[0],
                    self.first_ip[1],
                    self.first_ip[2],
                    self.first_ip[3],
                ))
            },
            IpNet::V6(_) => {
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&self.first_ip);
                IpAddr::V6(Ipv6Addr::from(bytes))
            }
        }
    }

    fn parse_ip_with_port(&self, ip_str: &str) -> (String, Option<u16>) {
        if ip_str.contains('[') && ip_str.contains(']') {
            let start = ip_str.find('[').unwrap();
            let end = ip_str.find(']').unwrap();
            let ip = &ip_str[start + 1..end];
            let port = if end + 1 < ip_str.len() && ip_str[end + 1..].starts_with(':') {
                ip_str[end + 2..].parse::<u16>().ok()
            } else {
                None
            };
            return (ip.to_string(), port);
        }

        if ip_str.contains(':') && is_ipv4(ip_str) {
            let parts: Vec<&str> = ip_str.split(':').collect();
            if parts.len() == 2 {
                return (parts[0].to_string(), parts[1].parse::<u16>().ok());
            }
        }

        (ip_str.to_string(), None)
    }

    fn append_ip(&mut self, ip: IpAddr, port: Option<u16>) {
        self.ips.push(IPWithPort { ip, port });
    }

    fn biguint_to_ipv4(&self, index: &BigUint) -> IpAddr {
        let bytes = index.to_bytes_be();
        let mut ip_bytes = [0u8; 4];
        let start = if bytes.len() >= 4 { bytes.len() - 4 } else { 0 };
        let copy_len = std::cmp::min(bytes.len(), 4);
        ip_bytes[4-copy_len..].copy_from_slice(&bytes[start..]);
        IpAddr::V4(Ipv4Addr::from(ip_bytes))
    }

    pub async fn load_ips_concurrent(&mut self, ip_text: &str, test_all: bool) -> io::Result<()> {
        debug!("开始解析 IP 数据");

        // 计算每个CIDR的选取数量
        let (cidr_counts, total_select) = self.calculate_select_count(ip_text);
        debug!("预计选取 IP 数量: {}", total_select);

        // 根据总量调整并发度
        let threshold = BigUint::from(2u32).pow(19); // 约500,000
        let ratio = if total_select > threshold {
            (threshold.clone() * BigUint::from(2u32).pow(20)) / total_select.clone()
        } else {
            BigUint::from(2u32).pow(20)
        };

        let ip_chunks: Vec<_> = ip_text
            .lines()
            .filter_map(|line| {
                let line = Self::remove_comments(line);
                if line.is_empty() {
                    None
                } else {
                    Some(line.split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>())
                }
            })
            .flatten()
            .collect();

        let mut handles = Vec::new();
        
        for (idx, chunk) in ip_chunks.iter().enumerate() {
            let chunk = chunk.to_string();
            let config = self.config.clone();
            let select_count = if idx < cidr_counts.len() {
                Some((cidr_counts[idx].clone() * ratio.clone() / 
                      BigUint::from(2u32).pow(20)).to_u32().unwrap_or(0))
            } else {
                None
            };
            
            let handle = tokio::spawn(async move {
                let mut ranges = IPRanges::new(config);
                ranges.parse_cidr(&chunk);
                if is_ipv4(&chunk) {
                    ranges.choose_ipv4(test_all, select_count);
                } else {
                    ranges.choose_ipv6(select_count);
                }
                ranges.ips
            });
            handles.push(handle);
        }

        let results = futures::stream::iter(handles)
            .buffer_unordered(num_cpus::get())
            .collect::<Vec<_>>()
            .await;

        for result in results {
            if let Ok(ips) = result {
                self.ips.extend(ips);
            }
        }

        debug!("IP 解析完成，共 {} 个有效 IP", self.ips.len());
        Ok(())
    }

    fn remove_comments(line: &str) -> &str {
        match line.find('#') {
            Some(idx) => &line[..idx],
            None => match line.find("//") {
                Some(idx) => &line[..idx],
                None => line,
            }
        }.trim()
    }

    pub fn get_ips(&self) -> Vec<IpAddr> {
        self.ips.iter().map(|ip_with_port| ip_with_port.ip).collect()
    }

    // 计算每个CIDR会选取的IP数量
    fn calculate_select_count(&self, ip_text: &str) -> (Vec<BigUint>, BigUint) {
        let mut cidr_counts = Vec::new();
        let mut total_select = BigUint::zero();
        
        // 第一遍：计算每个CIDR的原始选取数量
        for line in ip_text.lines() {
            let line = Self::remove_comments(line);
            if line.is_empty() {
                continue;
            }

            for ip_str in line.split(',') {
                let ip_str = ip_str.trim();
                let select_count = if let Ok(network) = IpNet::from_str(ip_str) {
                    match network {
                        IpNet::V4(net) => {
                            if net.prefix_len() == 32 {
                                BigUint::one()
                            } else {
                                let capacity = BigUint::from(2u32).pow(u32::from(32 - net.prefix_len()));
                                if self.config.test_all {
                                    capacity
                                } else if let Some(amount) = &self.config.ipv4_amount {
                                    let amount = BigUint::from(*amount);
                                    if amount >= capacity {
                                        capacity
                                    } else {
                                        amount
                                    }
                                } else {
                                    (capacity + BigUint::from(63u32)) / BigUint::from(64u32)
                                }
                            }
                        },
                        IpNet::V6(net) => {
                            if net.prefix_len() == 128 {
                                BigUint::one()
                            } else {
                                let target = match &self.config.ipv6_amount {
                                    Some(amount) => BigUint::from(parse_test_amount(&amount.to_string(), false)),
                                    None => match self.config.ipv6_num_mode.as_deref() {
                                        Some("more") => BigUint::from(2u32).pow(18u32), // 262144
                                        Some("lots") => BigUint::from(2u32).pow(16u32), // 65536
                                        Some("many") => BigUint::from(2u32).pow(12u32), // 4096
                                        Some("some") => BigUint::from(2u32).pow(8u32),  // 256
                                        _ => BigUint::from(2u32).pow(8u32)  // 默认256
                                    }
                                };
                                
                                let capacity = BigUint::from(2u32).pow(u32::from(128 - net.prefix_len()));
                                if target >= capacity {
                                    capacity
                                } else {
                                    target
                                }
                            }
                        }
                    }
                } else {
                    BigUint::one()  // 单个IP
                };
                
                cidr_counts.push(select_count.clone());
                total_select += select_count;
            }
        }

        // 如果总量超过50万，按比例调整每个CIDR的选取数量
        let threshold = BigUint::from(500_000u32);
        if total_select > threshold {
            let ratio = threshold.clone() * BigUint::from(1_000_000u32) / total_select.clone();
            for count in &mut cidr_counts {
                *count = (count.clone() * ratio.clone()) / BigUint::from(1_000_000u32);
                if *count == BigUint::zero() {
                    *count = BigUint::one(); // 确保每个CIDR至少选一个IP
                }
            }
            // 重新计算总量
            total_select = cidr_counts.iter().fold(BigUint::zero(), |acc, x| acc + x);
        }
        
        (cidr_counts, total_select)
    }
}

pub async fn load_ip_ranges_concurrent(config: &Config) -> io::Result<Vec<IpAddr>> {
    debug!("开始加载 IP 文件");
    
    let mut ranges = IPRanges::new(config.clone());
    let test_all = config.is_test_all();
    
    if !config.ip_text.is_empty() {
        debug!("使用命令行指定的 IP: {}", config.ip_text);
        ranges.load_ips_concurrent(&config.ip_text, test_all).await?;
    } else {
        debug!("尝试读取 IP 文件: {}", &config.ip_file);
        
        match std::fs::read_to_string(&config.ip_file) {
            Ok(content) => {
                debug!("成功读取 IP 文件，大小: {} bytes", content.len());
                if content.trim().is_empty() {
                    debug!("IP 文件内容为空");
                    return Ok(Vec::new());
                }
                ranges.load_ips_concurrent(&content, test_all).await?;
            }
            Err(e) => {
                debug!("读取 IP 文件失败: {}", e);
                return Ok(Vec::new());
            }
        }
    }
    
    Ok(ranges.get_ips())
}

fn is_ipv4(ip: &str) -> bool {
    ip.contains('.')
}

// 初始化随机数种子
pub fn init_rand_seed() {
    let mut rng = rand::thread_rng();
    rng.gen::<u64>();
}