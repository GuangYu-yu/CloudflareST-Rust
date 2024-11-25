use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::io;
use ipnet::IpNet;
use rand::Rng;
use crate::types::Config;
use futures::StreamExt;
use num_cpus;

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

struct IPRanges {
    ips: Vec<IpAddr>,
    mask: String,
    first_ip: Vec<u8>,
    ip_net: IpNet,
}

impl IPRanges {
    fn new() -> Self {
        Self {
            ips: Vec::new(),
            mask: String::new(),
            first_ip: Vec::new(),
            ip_net: "0.0.0.0/0".parse().unwrap(),
        }
    }

    // 如果是单独 IP 则加上子网掩码，反之则获取子网掩码(r.mask)
    fn fix_ip(&mut self, ip: &str) -> String {
        // 如果不含有 '/' 则代表不是 IP 段，而是一个单独的 IP，因此需要加上 /32 /128 子网掩码
        if !ip.contains('/') {
            self.mask = if is_ipv4(ip) {
                String::from("/32")
            } else {
                String::from("/128")
            };
            format!("{}{}", ip, self.mask)
        } else {
            self.mask = ip.split('/').nth(1).unwrap().to_string();
            ip.to_string()
        }
    }

    // 解析 IP 段，获得 IP、IP 范围、子网掩码
    fn parse_cidr(&mut self, ip: &str) {
        let ip = self.fix_ip(ip);
        self.ip_net = ip.parse().unwrap();
        self.first_ip = match self.ip_net.addr() {
            IpAddr::V4(ipv4) => ipv4.octets().to_vec(),
            IpAddr::V6(ipv6) => ipv6.octets().to_vec(),
        };
    }

    fn append_ipv4(&mut self, d: u8) {
        let ip = Ipv4Addr::new(
            self.first_ip[0],
            self.first_ip[1], 
            self.first_ip[2],
            d
        );
        self.ips.push(IpAddr::V4(ip));
    }

    fn append_ip(&mut self, ip: IpAddr) {
        self.ips.push(ip);
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

    fn choose_ipv4(&mut self, test_all: bool) {
        if self.mask == "/32" {
            // 单个 IP 则无需随机，直接加入自身即可
            self.append_ip(IpAddr::V4(Ipv4Addr::new(
                self.first_ip[0],
                self.first_ip[1],
                self.first_ip[2],
                self.first_ip[3],
            )));
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

    fn choose_ipv6(&mut self) {
        if self.mask == "/128" {
            // 单个 IP 则无需随机，直接加入自身即可
            let mut ip_bytes = [0u16; 8];
            for i in 0..8 {
                ip_bytes[i] = ((self.first_ip[i*2] as u16) << 8) | (self.first_ip[i*2+1] as u16);
            }
            self.append_ip(IpAddr::V6(Ipv6Addr::new(
                ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3],
                ip_bytes[4], ip_bytes[5], ip_bytes[6], ip_bytes[7],
            )));
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
            )));

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

    pub async fn load_ips_concurrent(&mut self, ip_text: &str, test_all: bool) -> io::Result<()> {
        let ip_chunks: Vec<_> = ip_text.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
            
        let mut handles = Vec::new();
        
        for chunk in ip_chunks {
            let chunk = chunk.to_string();
            let test_all = test_all; // 克隆 test_all 标志
            let handle = tokio::spawn(async move {
                let mut ranges = IPRanges::new();
                ranges.parse_cidr(&chunk);
                if is_ipv4(&chunk) {
                    ranges.choose_ipv4(test_all); // 使用 test_all 标志
                } else {
                    ranges.choose_ipv6();
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
        
        Ok(())
    }
}

pub async fn load_ip_ranges_concurrent(config: &Config) -> io::Result<Vec<IpAddr>> {
    let mut ranges = IPRanges::new();
    let test_all = config.is_test_all();
    
    if !config.ip_text.is_empty() {
        ranges.load_ips_concurrent(&config.ip_text, test_all).await?;
    } else {
        let file_path = if config.ip_file.is_empty() { "ip.txt" } else { &config.ip_file };
        if let Ok(content) = std::fs::read_to_string(file_path) {
            ranges.load_ips_concurrent(&content, test_all).await?;
        }
    }
    
    Ok(ranges.ips)
}