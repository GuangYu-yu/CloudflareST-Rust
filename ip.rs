use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::str::FromStr;
use ipnet::IpNet;
use rand::Rng;

// 全局变量
pub static mut TEST_ALL: bool = false;
pub static mut IP_FILE: &str = "ip.txt";
pub static mut IP_TEXT: &str = "";

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
            IpAddr::V4(ip) => ip.octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
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
        let min_ip = self.first_ip[3] & self.ip_net.netmask().octets()[3];
        
        let hosts = match self.ip_net.prefix_len() {
            32 => 0,
            24 => 255,
            n => {
                let h = (1u32 << (32 - n)) - 1;
                if h > 255 { 255 } else { h as u8 }
            }
        };

        (min_ip, hosts)
    }

    fn choose_ipv4(&mut self) {
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
            unsafe {
                if TEST_ALL {
                    // 如果是测速全部 IP
                    for i in 0..=hosts {
                        self.append_ipv4(i + min_ip);
                    }
                } else {
                    // 随机 IP 的最后一段 0.0.0.X
                    self.append_ipv4(min_ip + rand_ip_end_with(hosts));
                }
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
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&self.first_ip);
            self.append_ip(IpAddr::V6(Ipv6Addr::from(ip_bytes)));
            return;
        }

        while self.ip_net.contains(&IpAddr::V6(Ipv6Addr::from_slice(&self.first_ip).unwrap())) {
            self.first_ip[15] = rand_ip_end_with(255);
            self.first_ip[14] = rand_ip_end_with(255);

            let mut target_ip = self.first_ip.clone();
            self.append_ip(IpAddr::V6(Ipv6Addr::from_slice(&target_ip).unwrap()));

            for i in (0..14).rev() {
                let temp_ip = self.first_ip[i];
                self.first_ip[i] = self.first_ip[i].wrapping_add(rand_ip_end_with(255));
                if self.first_ip[i] >= temp_ip {
                    break;
                }
            }
        }
    }
}

pub fn load_ip_ranges() -> Vec<IpAddr> {
    let mut ranges = IPRanges::new();
    
    unsafe {
        if !IP_TEXT.is_empty() {
            // 从参数中获取 IP 段数据
            for ip in IP_TEXT.split(',') {
                let ip = ip.trim();
                if ip.is_empty() {
                    continue;
                }
                ranges.parse_cidr(ip);
                if is_ipv4(ip) {
                    ranges.choose_ipv4();
                } else {
                    ranges.choose_ipv6();
                }
            }
        } else {
            // 从文件中获取 IP 段数据
            let file_path = if IP_FILE.is_empty() { "ip.txt" } else { IP_FILE };
            if let Ok(lines) = read_lines(file_path) {
                for line in lines.flatten() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    ranges.parse_cidr(line);
                    if is_ipv4(&line) {
                        ranges.choose_ipv4();
                    } else {
                        ranges.choose_ipv6();
                    }
                }
            }
        }
    }

    ranges.ips
}

// 辅助函数:读取文件行
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path> {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
} 