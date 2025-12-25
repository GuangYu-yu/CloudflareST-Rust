use std::fs::File;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering},
};
use std::thread;

use crate::args::Args;

/// IPv4/IPv6 CIDR 网络块
#[derive(Clone, Copy)]
pub(crate) enum IpCidr {
    V4(Ipv4Addr, u8),
    V6(Ipv6Addr, u8),
}

impl IpCidr {
    fn parts(&self) -> (u128, u8, u8, u128) {
        match self {
            IpCidr::V4(ip, len) => (u32::from(*ip) as u128, *len, 32, u32::MAX as u128),
            IpCidr::V6(ip, len) => (u128::from(*ip), *len, 128, u128::MAX),
        }
    }

    /// 计算地址范围，返回 (起始地址, 结束地址)
    pub(crate) fn range_u128(&self) -> (u128, u128) {
        let (val, len, max_bits, full_mask) = self.parts();
        let host_bits = max_bits - len;

        if host_bits >= max_bits {
            return (0, full_mask);
        }

        let mask = full_mask << host_bits & full_mask;
        let start = val & mask;
        let end = start | (!mask & full_mask);
        
        (start, end)
    }

    pub(crate) fn prefix_len(&self) -> u8 {
        match self {
            IpCidr::V4(_, len) | IpCidr::V6(_, len) => *len,
        }
    }

    pub(crate) fn is_single_host(&self) -> bool {
        matches!(self, IpCidr::V4(_, 32) | IpCidr::V6(_, 128))
    }

    pub(crate) fn to_ipaddr(self) -> IpAddr {
        let (start, _) = self.range_u128();
        match self {
            IpCidr::V4(..) => IpAddr::V4(Ipv4Addr::from(start as u32)),
            IpCidr::V6(..) => IpAddr::V6(Ipv6Addr::from(start)),
        }
    }

    /// 解析 CIDR 格式字符串
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return None;
        }

        let ip = IpAddr::from_str(parts[0]).ok()?;
        let prefix = parts[1].parse::<u8>().ok()?;

        match ip {
            IpAddr::V4(v4) if prefix <= 32 => Some(IpCidr::V4(v4, prefix)),
            IpAddr::V6(v6) if prefix <= 128 => Some(IpCidr::V6(v6, prefix)),
            _ => None,
        }
    }
}

/// IP 地址缓冲区
pub(crate) struct IpBuffer {
    total_expected: usize,
    segments: AtomicPtr<Vec<Arc<IpSegment>>>,
    cursor: AtomicUsize,
    active_count: AtomicUsize,
    initial_len: usize,
    reading_threads: AtomicUsize,
    tcp_port: u16,
}

unsafe impl Send for IpBuffer {}
unsafe impl Sync for IpBuffer {}

pub(crate) enum IpSegment {
    Static {
        ips: Vec<SocketAddr>,
        cursor: AtomicUsize,
        exhausted_notified: AtomicBool,
    },
    Generator {
        cidr: Arc<CidrState>,
        exhausted_notified: AtomicBool,
    },
}

impl IpSegment {
    fn next_ip(&self, tcp_port: u16) -> Option<SocketAddr> {
        match self {
            IpSegment::Static { ips, cursor, .. } => {
                let idx = cursor.fetch_add(1, Ordering::Relaxed);
                ips.get(idx).copied()
            }
            IpSegment::Generator { cidr, .. } => cidr.next_ip(tcp_port),
        }
    }

    fn is_exhausted(&self) -> bool {
        match self {
            IpSegment::Static { ips, cursor, .. } => {
                cursor.load(Ordering::Relaxed) >= ips.len()
            }
            IpSegment::Generator { cidr, .. } => cidr.is_exhausted(),
        }
    }

    fn mark_dead_once(&self) -> bool {
        match self {
            IpSegment::Static { exhausted_notified, .. } | 
            IpSegment::Generator { exhausted_notified, .. } => {
                exhausted_notified.compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed).is_ok()
            }
        }
    }
}

/// CIDR 网络扫描状态
pub(crate) struct CidrState {
    id: usize,
    network: IpCidr,
    total_count: usize,
    interval_size: u128,
    start: u128,
    end: u128,
    index_counter: AtomicUsize,
    is_finished: AtomicBool,
}

impl CidrState {
    /// SplitMix64
    fn splitmix_u64(index: u64, seed_offset: u64) -> u64 {
        let mut z = index ^ seed_offset;
        z ^= z >> 33;
        z.wrapping_mul(0x9E3779B97F4A7C15)
    }

    pub(crate) fn new(id: usize, network: IpCidr, count: usize, start: u128, end: u128, interval_size: u128) -> Self {
        Self {
            id,
            network,
            total_count: count,
            interval_size,
            start,
            end,
            index_counter: AtomicUsize::new(0),
            is_finished: AtomicBool::new(false),
        }
    }

    /// 生成下一个随机 IP 地址
    fn next_ip(&self, tcp_port: u16) -> Option<SocketAddr> {
        let current_index = self.index_counter.fetch_add(1, Ordering::Relaxed);

        if current_index >= self.total_count {
            self.is_finished.store(true, Ordering::Relaxed);
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
                self.id as u64 ^ (&self.id as *const usize as u64)
            );

            (mixed_val as u128) % actual_interval_size
        };

        let random_ip = interval_start + random_offset;

        let ip_addr = match self.network {
            IpCidr::V4(..) => IpAddr::V4(Ipv4Addr::from(random_ip as u32)),
            IpCidr::V6(..) => IpAddr::V6(Ipv6Addr::from(random_ip)),
        };

        Some(SocketAddr::new(ip_addr, tcp_port))
    }

    fn is_exhausted(&self) -> bool {
        self.is_finished.load(Ordering::Relaxed)
    }
}

impl IpBuffer {
    pub(crate) fn new(
        cidr_states: Vec<CidrState>,
        single_ips: Vec<SocketAddr>,
        total_expected: usize,
        tcp_port: u16,
    ) -> Self {
        let mut segments: Vec<Arc<IpSegment>> = Vec::new();

        const CHUNK_SIZE: usize = 1024;

        if !single_ips.is_empty() {
            for chunk in single_ips.chunks(CHUNK_SIZE) {
                segments.push(Arc::new(IpSegment::Static {
                    ips: chunk.to_vec(),
                    cursor: AtomicUsize::new(0),
                    exhausted_notified: AtomicBool::new(false),
                }));
            }
        }

        for cidr in cidr_states {
            segments.push(Arc::new(IpSegment::Generator {
                cidr: Arc::new(cidr),
                exhausted_notified: AtomicBool::new(false),
            }));
        }

        let initial_len = segments.len();
        let segments_arc = Arc::new(segments);

        Self {
            total_expected,
            segments: AtomicPtr::new(Arc::into_raw(segments_arc) as *mut _),
            cursor: AtomicUsize::new(0),
            active_count: AtomicUsize::new(initial_len),
            initial_len,
            reading_threads: AtomicUsize::new(0),
            tcp_port,
        }
    }

    /// 弹出一个 IP 地址，优先处理单个 IP，其次轮询 CIDR 块
    pub(crate) fn pop(&self) -> Option<SocketAddr> {
        loop {
            if self.active_count.load(Ordering::Acquire) == 0 {
                return None;
            }

            self.reading_threads.fetch_add(1, Ordering::Acquire);

            let ptr = self.segments.load(Ordering::Acquire);
            if ptr.is_null() {
                self.reading_threads.fetch_sub(1, Ordering::Release);
                return None;
            }

            let current_vec = unsafe {
                Arc::increment_strong_count(ptr);
                Arc::from_raw(ptr)
            };

            self.reading_threads.fetch_sub(1, Ordering::Release);

            let segments_len = current_vec.len();
            if segments_len == 0 {
                return None;
            }

            let start_idx = self.cursor.fetch_add(1, Ordering::Relaxed);

            for i in 0..segments_len {
                let idx = (start_idx + i) % segments_len;
                let segment = &current_vec[idx];

                if let Some(ip) = segment.next_ip(self.tcp_port) {
                    return Some(ip);
                }

                if segment.is_exhausted() && segment.mark_dead_once() {
                    let new_count = self.active_count.fetch_sub(1, Ordering::SeqCst) - 1;
                    
                    if new_count <= self.initial_len / 2 {
                        self.try_reaping(ptr, &current_vec);
                    }
                }
            }

            if self.active_count.load(Ordering::Acquire) == 0 {
                return None;
            }
        }
    }

    fn try_reaping(&self, old_ptr: *mut Vec<Arc<IpSegment>>, current: &[Arc<IpSegment>]) {
        let new_vec = current.iter().filter(|s| !s.is_exhausted()).cloned().collect::<Vec<_>>();

        let new_arc = Arc::new(new_vec);
        let new_ptr = Arc::into_raw(new_arc) as *mut _;

        if self.segments.compare_exchange(
            old_ptr,
            new_ptr,
            Ordering::AcqRel,
            Ordering::Relaxed
        ).is_ok() {
            while self.reading_threads.load(Ordering::Acquire) > 0 {
                thread::yield_now();
            }
            unsafe { Arc::from_raw(old_ptr); }
        } else {
            unsafe { Arc::from_raw(new_ptr); }
        }
    }

    pub(crate) fn total_expected(&self) -> usize {
        self.total_expected
    }
}

impl Drop for IpBuffer {
    fn drop(&mut self) {
        let ptr = self.segments.load(Ordering::Acquire);
        if !ptr.is_null() {
            unsafe { Arc::from_raw(ptr); }
        }
    }
}

/// 收集 IP/CIDR 来源
pub(crate) fn collect_ip_sources(ip_text: &str, ip_file: &str) -> Vec<String> {
    let clean = |s: &str| {
        let s = s.trim();
        (!s.is_empty() && !s.starts_with('#') && !s.starts_with("//")).then(|| s.to_string())
    };

    let mut sources: Vec<_> = ip_text.split(',').filter_map(clean).collect();

    if !ip_file.is_empty() && let Ok(file) = File::open(ip_file) {
        sources.extend(io::BufReader::new(file).lines().map_while(Result::ok).filter_map(|l| clean(&l)));
    }

    sources.sort_unstable();
    sources.dedup();
    
    if sources.is_empty() {
        crate::error_and_exit(format_args!("未获取到任何 IP 或 CIDR"));
    }

    sources
}

/// 统一的解析结果，包含IP信息和自定义计数
#[derive(Clone)]
struct ParsedIpInfo {
    result: IpParseResult,
    custom_count: Option<u128>,
}

/// 统一解析IP范围和计数信息
fn parse_ip_info(ip_range: &str) -> ParsedIpInfo {
    let (ip_part, custom_count) = if let Some((ip_part, count_str)) = ip_range.split_once('=') {
        let count = count_str.trim().parse::<u128>().ok().filter(|&n| n > 0);
        (ip_part.trim(), count)
    } else {
        (ip_range, None)
    };
    
    ParsedIpInfo {
        result: parse_ip_with_port(ip_part),
        custom_count,
    }
}

#[derive(Clone)]
enum IpParseResult {
    SocketAddr(SocketAddr),
    Network(IpCidr),
    Invalid,
}

fn parse_ip_with_port(ip_str: &str) -> IpParseResult {
    if let Ok(socket_addr) = SocketAddr::from_str(ip_str) {
        return IpParseResult::SocketAddr(socket_addr);
    }

    if let Ok(ip_addr) = IpAddr::from_str(ip_str) {
        let network = match ip_addr {
            IpAddr::V4(v4) => IpCidr::V4(v4, 32),
            IpAddr::V6(v6) => IpCidr::V6(v6, 128),
        };
        return IpParseResult::Network(network);
    }

    if let Some(network) = IpCidr::parse(ip_str) {
        return IpParseResult::Network(network);
    }

    IpParseResult::Invalid
}

/// 处理 IP 来源
pub(crate) fn process_ip_sources(ip_sources: Vec<String>, config: &Args) -> (Vec<SocketAddr>, Vec<CidrState>, usize) {
    let mut single_ips = Vec::new();
    let mut cidr_info = Vec::new();
    let mut total_expected = 0;

    for ip_range in ip_sources {
        let ip_info = parse_ip_info(&ip_range);

        match &ip_info.result {
            IpParseResult::SocketAddr(socket_addr) => {
                single_ips.push(*socket_addr);
                total_expected += 1;
            }
            IpParseResult::Network(network) => {
                if network.is_single_host() {
                    single_ips.push(SocketAddr::new(network.to_ipaddr(), config.tcp_port));
                    total_expected += 1;
                } else {
                    let count = calculate_ip_count(&ip_info.result, ip_info.custom_count, config.test_all_ipv4);
                    let (start, end) = network.range_u128();

                    let range_size = (end - start).saturating_add(1);

                    let adjusted_count = count.min(range_size) as usize;

                    let interval_size = if adjusted_count > 0 {
                        (range_size / adjusted_count as u128).max(1)
                    } else {
                        1
                    };

                    total_expected += adjusted_count;
                    cidr_info.push((*network, adjusted_count, start, end, interval_size));
                }
            }
            IpParseResult::Invalid => {}
        }
    }

    let cidr_states: Vec<_> = cidr_info
        .into_iter()
        .enumerate()
        .map(|(id, (net, count, start, end, size))| CidrState::new(id, net, count, start, end, size))
        .collect();

    (single_ips, cidr_states, total_expected)
}

/// 计算采样 IP 数量
fn calculate_ip_count(parsed_result: &IpParseResult, custom_count: Option<u128>, test_all_ipv4: bool) -> u128 {
    match parsed_result {
        IpParseResult::SocketAddr(_) => {
            1
        }
        IpParseResult::Network(network) => {
            if network.is_single_host() {
                return 1;
            }

            let prefix = network.prefix_len();
            let is_ipv4 = matches!(network, IpCidr::V4(_, _));

            if let Some(count) = custom_count {
                return count;
            }

            if is_ipv4 && test_all_ipv4 {
                return if prefix < 32 {
                    1u128 << (32 - prefix)
                } else {
                    1
                };
            }

            calculate_sample_count(prefix, is_ipv4)
        }
        IpParseResult::Invalid => {
            0
        }
    }
}

/// 根据前缀长度计算采样数量
pub(crate) fn calculate_sample_count(prefix: u8, is_ipv4: bool) -> u128 {
    let max_bits: u8 = if is_ipv4 { 31 } else { 127 };
    let host_bits = max_bits.saturating_sub(prefix);
    let sample_exp = host_bits.min(18).saturating_sub(3);
    1u128 << sample_exp
}
