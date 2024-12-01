use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::io;
use ipnet::IpNet;
use rand::Rng;
use crate::types::{Config, parse_test_amount};
use futures::StreamExt;
use num_cpus;
use crate::debug_log;
#[cfg(feature = "debug")]
use tracing;
use std::str::FromStr;
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive};
use sha3::{Shake256, digest::{Update, ExtendableOutput, XofReader}};

const DEFAULT_IPV6_TEST_COUNT: u32 = 1u32 << 8;  // 256 = 2^8

#[derive(Clone)]
pub struct IPWithPort {
    pub ip: IpAddr,
    pub port: Option<u16>,
}

impl IPWithPort {
    pub fn get_port(&self, config_port: u16) -> u16 {
        self.port.unwrap_or(
            if config_port != 0 { 
                config_port 
            } else { 
                443 
            }
        )
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
            let prefix_len = self.get_prefix_len(&network);
            self.mask = format!("/{}", prefix_len);
            self.first_ip = self.get_network_bytes(&network);
        }
    }

    fn calculate_select_count(&self, ip_text: &str) -> Vec<BigUint> {
        debug_log!("开始计算选择数量");
        let mut cidr_counts = Vec::new();
        
        for line in ip_text.lines() {
            let line = Self::remove_comments(line);
            if line.is_empty() { continue; }

            for ip_str in line.split(',') {
                let ip_str = ip_str.trim();
                debug_log!("处理 CIDR: {}", ip_str);
                
                let select_count = if let Ok(network) = IpNet::from_str(ip_str) {
                    match network {
                        IpNet::V4(net) => {
                            debug_log!("IPv4 CIDR: {}, 前缀长度: {}", net, net.prefix_len());
                            if net.prefix_len() == 32 {
                                debug_log!("单个 IPv4 地址");
                                BigUint::one()
                            } else if self.config.test_all {
                                // 使用 all4 时直接返回最大值
                                BigUint::from(u32::MAX)
                            } else if let Some(amount) = &self.config.ipv4_amount {
                                BigUint::from(parse_test_amount(&amount.to_string(), true))
                            } else {
                                // 每64个IP选1个
                                let available_ips = 2u32.pow((32 - net.prefix_len()) as u32);
                                BigUint::from((available_ips + 63) / 64)
                            }
                        },
                        IpNet::V6(net) => {
                            debug_log!("IPv6 CIDR: {}, 前缀长度: {}", net, net.prefix_len());
                            if net.prefix_len() == 128 {
                                debug_log!("单个 IPv6 地址");
                                BigUint::one()
                            } else {
                                if let Some(amount) = &self.config.ipv6_amount {
                                    BigUint::from(parse_test_amount(&amount.to_string(), false))
                                } else if let Some(mode) = self.config.ipv6_num_mode.as_deref() {
                                    match mode {
                                        "more" => BigUint::from(1u32 << 18), // 2^18 = 262144
                                        "lots" => BigUint::from(1u32 << 16), // 2^16 = 65536
                                        "many" => BigUint::from(1u32 << 12), // 2^12 = 4096
                                        "some" => BigUint::from(1u32 << 10), // 2^10 = 1024
                                        _ => BigUint::from(DEFAULT_IPV6_TEST_COUNT)
                                    }
                                } else {
                                    // 默认每个 CIDR 测试 256 个 IP
                                    BigUint::from(DEFAULT_IPV6_TEST_COUNT)
                                }
                            }
                        }
                    }
                } else {
                    debug_log!("无效的 CIDR 格式: {}", ip_str);
                    BigUint::one()
                };
                
                cidr_counts.push(select_count);
            }
        }
        
        cidr_counts
    }

    pub async fn load_ips_concurrent(&mut self, ip_text: &str, test_all: bool) -> io::Result<()> {
        debug_log!("开始解析 IP 数据");

        let cidr_counts = self.calculate_select_count(ip_text);
        let _total_expected = cidr_counts.iter().sum::<BigUint>();
        debug_log!("预计选取 IP 数量: {}", _total_expected);

        let ip_chunks: Vec<_> = ip_text
            .lines()
            .filter_map(|line| {
                let line = Self::remove_comments(line);
                if line.is_empty() {
                    None
                } else {
                    debug_log!("处理行: {}", line);
                    Some(line)
                }
            })
            .flat_map(|line| {
                let chunks: Vec<_> = line.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                debug_log!("分割后的 CIDR: {:?}", chunks);
                chunks
            })
            .collect();

        debug_log!("处理的 CIDR 总数: {}", ip_chunks.len());

        let mut handles = Vec::new();
        
        for (idx, chunk) in ip_chunks.iter().enumerate() {
            let chunk = chunk.to_string();
            let config = self.config.clone();
            let select_count = if idx < cidr_counts.len() {
                debug_log!("CIDR {}: 目标生成数量 {}", chunk, cidr_counts[idx]);
                Some(cidr_counts[idx].clone())
            } else {
                None
            };
            
            let handle = tokio::spawn(async move {
                let mut ranges = IPRanges::new(config);
                ranges.parse_cidr(&chunk);
                match ranges.ip_net {
                    IpNet::V4(_) => {
                        ranges.choose_ipv4(test_all, select_count);
                    },
                    IpNet::V6(_) => {
                        ranges.choose_ipv6(select_count);
                    }
                }
                ranges.ips
            });
            
            handles.push(handle);
        }

        let results = futures::stream::iter(handles)
            .buffer_unordered(num_cpus::get())
            .collect::<Vec<_>>()
            .await;

        let mut _total_generated = 0;
        for result in results {
            if let Ok(ips) = result {
                debug_log!("添加 CIDR 生成的 IP 数量: {}", ips.len());
                _total_generated += ips.len();
                self.ips.extend(ips);
            }
        }

        debug_log!("IP 解析完成，共 {} 个有效 IP", self.ips.len());
        Ok(())
    }

    fn append_ip(&mut self, ip: IpAddr, port: Option<u16>) {
        self.ips.push(IPWithPort { ip, port });
    }

    fn current_ip(&self) -> IpAddr {
        match self.ip_net {
            IpNet::V4(net) => IpAddr::V4(net.addr()),
            IpNet::V6(net) => IpAddr::V6(net.addr()),
        }
    }

    fn get_prefix_len(&self, network: &IpNet) -> u8 {
        network.prefix_len()
    }

    fn get_network_bytes(&self, network: &IpNet) -> Vec<u8> {
        match network {
            IpNet::V4(net) => {
                let bytes = net.addr().octets();
                bytes.to_vec()
            },
            IpNet::V6(net) => {
                let bytes = net.addr().octets();
                bytes.to_vec()
            }
        }
    }

    fn get_ips(&self) -> Vec<IpAddr> {
        self.ips.iter().map(|ip| ip.ip).collect()
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
                let port = parts[1].parse::<u16>().ok();
                return (parts[0].to_string(), port);
            }
        }

        (ip_str.to_string(), None)
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

    fn choose_random_ips_o1(&mut self, target_amount: BigUint, _is_ipv4: bool) -> Vec<IpAddr> {
        debug_log!("开始随机选择 IP，目标数量: {}, 是否IPv4: {}", target_amount, _is_ipv4);
        
        let mut rng = rand::thread_rng();
        let target = target_amount.to_usize().unwrap_or(0);
        let seed = rng.gen::<u64>();
        
        debug_log!("初始化随机数生成器，种子: {}", seed);
        
        match &self.ip_net {
            IpNet::V4(net) => {
                debug_log!("处理 IPv4 网段: {}", net);
                let mut ip_bytes = net.network().octets().to_vec();
                let prefix_len = BigUint::from(net.prefix_len());
                let total_bits = BigUint::from(32u32);
                let variable_bits = &total_bits - &prefix_len;
                
                debug_log!("网段前缀长度: {}, 可变位数: {}", prefix_len, variable_bits);
                debug_log!("初始 IP 字节: {:?}", ip_bytes);
                
                let start_byte = prefix_len.to_usize().unwrap() / 8;
                let bit_offset = prefix_len.to_usize().unwrap() % 8;
                let bytes_needed = ((variable_bits.clone() + BigUint::from(7u32)) / BigUint::from(8u32)).to_usize().unwrap();
                
                let mut i: u64 = 0;
                while self.ips.len() < target {
                    // 为每个IP生成新的哈希
                    let mut hasher = Shake256::default();
                    hasher.update(&seed.to_be_bytes());
                    hasher.update(&i.to_be_bytes());
                    let mut reader = hasher.finalize_xof();
                    
                    // 生成所需字节数的随机数据
                    let mut random_bytes = vec![0u8; bytes_needed];
                    reader.read(&mut random_bytes);
                    
                    // 处理第一个可能不完整的字节
                    if bit_offset != 0 {
                        let mask = !0u8 >> bit_offset;
                        let preserved = ip_bytes[start_byte] & !mask;
                        let variable = random_bytes[0] & mask;
                        ip_bytes[start_byte] = preserved | variable;
                    }
                    
                    // 处理剩余的完整字节
                    let start_pos = if bit_offset != 0 { 1 } else { 0 };
                    for j in 0..bytes_needed - start_pos {
                        let byte_pos = start_byte + start_pos + j;
                        if byte_pos < ip_bytes.len() {
                            ip_bytes[byte_pos] = random_bytes[j + start_pos];
                        }
                    }
                    
                    // 构造IP并添加
                    let ip = IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(&ip_bytes[..4]).unwrap()));
                    debug_log!("生成的IP: {}", ip);
                    self.append_ip_with_limit(ip, None);
                    i += 1;
                }
            },
            IpNet::V6(net) => {
                debug_log!("处理 IPv6 网段: {}", net);
                let mut ip_bytes = net.network().octets().to_vec();
                let prefix_len = BigUint::from(net.prefix_len());
                let total_bits = BigUint::from(128u32);
                let variable_bits = &total_bits - &prefix_len;
                
                debug_log!("网段前缀长度: {}, 可变位数: {}", prefix_len, variable_bits);
                debug_log!("初始网络字节: {:?}", ip_bytes);
                
                let start_byte = prefix_len.to_usize().unwrap() / 8;
                let bit_offset = prefix_len.to_usize().unwrap() % 8;
                let bytes_needed = ((variable_bits.clone() + BigUint::from(7u32)) / BigUint::from(8u32)).to_usize().unwrap();
                
                let mut i: u64 = 0;
                while self.ips.len() < target {
                    let mut hasher = Shake256::default();
                    hasher.update(&seed.to_be_bytes());
                    hasher.update(&i.to_be_bytes());
                    let mut reader = hasher.finalize_xof();
                    
                    let mut random_bytes = vec![0u8; bytes_needed];
                    reader.read(&mut random_bytes);
                    
                    if bit_offset != 0 {
                        let mask = !0u8 >> bit_offset;
                        let preserved = ip_bytes[start_byte] & !mask;
                        let variable = random_bytes[0] & mask;
                        ip_bytes[start_byte] = preserved | variable;
                    }
                    
                    let start_pos = if bit_offset != 0 { 1 } else { 0 };
                    for j in 0..bytes_needed - start_pos {
                        let byte_pos = start_byte + start_pos + j;
                        if byte_pos < ip_bytes.len() {
                            ip_bytes[byte_pos] = random_bytes[j + start_pos];
                        }
                    }
                    
                    let ip = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&ip_bytes[..16]).unwrap()));
                    debug_log!("生成的IP: {}", ip);
                    self.append_ip_with_limit(ip, None);
                    i += 1;
                }
            }
        }
        
        debug_log!("IP生成完成，最终生成数量: {}", self.ips.len());
        self.get_ips()
    }
    
    fn choose_ipv4(&mut self, test_all: bool, target_amount: Option<BigUint>) {
        if self.mask == "/32" {
            self.append_ip_with_limit(self.current_ip(), None);
            return;
        }

        let target = if test_all {
            BigUint::from(u32::MAX)
        } else if let Some(amount) = target_amount {
            amount
        } else {
            // 每64个IP选1个
            let prefix_len = self.ip_net.prefix_len();
            let available_ips = 2u32.pow((32 - prefix_len) as u32);
            BigUint::from((available_ips + 63) / 64)
        };

        let ips = self.choose_random_ips_o1(target, true);
        for ip in ips {
            self.append_ip_with_limit(ip, None);
        }
    }

    fn choose_ipv6(&mut self, target_amount: Option<BigUint>) {
        if self.mask == "/128" {
            self.append_ip_with_limit(self.current_ip(), None);
            return;
        }

        let target = target_amount.unwrap_or_else(|| BigUint::from(DEFAULT_IPV6_TEST_COUNT));
        debug_log!("IPv6 目标生成数量: {}", target);

        // 清空之前的 IP
        self.ips.clear();
        
        let _ips = self.choose_random_ips_o1(target, false);
        debug_log!("生成 IP 数量: {}", _ips.len());
        
        debug_log!("最终保留 IP 数量: {}", self.ips.len());
    }

    fn append_ip_with_limit(&mut self, ip: IpAddr, port: Option<u16>) {
        if self.ips.len() >= self.config.max_ip_count {
            // 超过阈值，随机丢弃一个已有的 IP
            let mut rng = rand::thread_rng();
            let remove_idx = rng.gen_range(0..self.ips.len());
            self.ips.swap_remove(remove_idx);
        }
        
        // 添加新 IP
        self.ips.push(IPWithPort { ip, port });
    }
}

pub async fn load_ip_ranges_concurrent(config: &Config) -> io::Result<Vec<IpAddr>> {
    debug_log!("开始加载 IP 文件");
    
    let mut ranges = IPRanges::new(config.clone());
    let test_all = config.is_test_all();
    
    if !config.ip_text.is_empty() {
        debug_log!("使用命令行指定的 IP: {}", config.ip_text);
        ranges.load_ips_concurrent(&config.ip_text, test_all).await?;
    } else {
        debug_log!("尝试读取 IP 文件: {}", &config.ip_file);
        
        match std::fs::read_to_string(&config.ip_file) {
            Ok(content) => {
                debug_log!("成功读取 IP 文件，大小: {} bytes", content.len());
                if content.trim().is_empty() {
                    debug_log!("IP 文件内容为空");
                    return Ok(Vec::new());
                }
                ranges.load_ips_concurrent(&content, test_all).await?;
            }
            Err(_e) => {
                debug_log!("读取 IP 文件失败: {}", _e);
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