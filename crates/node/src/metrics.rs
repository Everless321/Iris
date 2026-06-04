// M7 节点资源监控采样。
//
// sysinfo 提供 CPU/RAM/swap/disk/load/process/uptime；网络速率自己算（两次采样差/秒）；
// TCP/UDP 连接数读 /proc/net/sockstat（Linux only，其他平台返回 0）。
// 采样周期 = heartbeat 周期（5s），首次采样 cpu_usage 因 sysinfo 需要两次 refresh 而为 0，
// 此次的网络速率也是 0；从第二次心跳起所有值真实。

use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Disks, Networks, System};

use iris_proto::control::NodeMetrics;

pub struct MetricsCollector {
    inner: Mutex<Inner>,
}

struct Inner {
    sys: System,
    networks: Networks,
    disks: Disks,
    // 上一次累计网络字节 + 时间，用于算 bps
    last_net_total_up: u64,
    last_net_total_down: u64,
    last_sample_at: Instant,
    // 静态信息缓存（只算一次）
    static_info: StaticInfo,
}

#[derive(Clone)]
struct StaticInfo {
    cpu_name: String,
    cpu_cores: u32,
    arch: String,
    os: String,
    kernel: String,
    virtualization: String,
}

impl MetricsCollector {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let static_info = compute_static_info(&sys);
        Self {
            inner: Mutex::new(Inner {
                sys,
                networks,
                disks,
                last_net_total_up: 0,
                last_net_total_down: 0,
                last_sample_at: Instant::now(),
                static_info,
            }),
        }
    }

    pub fn sample(&self) -> NodeMetrics {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return NodeMetrics::default(),
        };
        g.sys.refresh_cpu_usage();
        g.sys.refresh_memory();
        g.sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        g.networks.refresh();
        g.disks.refresh();

        // CPU 使用率 = 各核心 usage 平均
        let cpus = g.sys.cpus();
        let cpu_usage = if cpus.is_empty() {
            0.0
        } else {
            cpus.iter().map(|c| c.cpu_usage() as f64).sum::<f64>() / cpus.len() as f64
        };

        // RAM / swap
        let ram_total = g.sys.total_memory();
        let ram_used = g.sys.used_memory();
        let swap_total = g.sys.total_swap();
        let swap_used = g.sys.used_swap();

        // load avg
        let load = System::load_average();

        // 磁盘：所有挂载点累加（避免按 mount 拆，简化前端）
        let mut disk_total: u64 = 0;
        let mut disk_used: u64 = 0;
        for d in g.disks.list() {
            let total = d.total_space();
            let avail = d.available_space();
            disk_total = disk_total.saturating_add(total);
            disk_used = disk_used.saturating_add(total.saturating_sub(avail));
        }

        // 网络速率：用累计字节差 / 时间差
        let mut total_up = 0u64;
        let mut total_down = 0u64;
        for (_, data) in g.networks.iter() {
            total_up = total_up.saturating_add(data.total_transmitted());
            total_down = total_down.saturating_add(data.total_received());
        }
        let now = Instant::now();
        let elapsed_secs = now.saturating_duration_since(g.last_sample_at).as_secs_f64();
        let (up_bps, down_bps) = if g.last_net_total_up == 0 || elapsed_secs < 0.1 {
            (0, 0)
        } else {
            let up = ((total_up.saturating_sub(g.last_net_total_up)) as f64 * 8.0 / elapsed_secs) as u64;
            let dn = ((total_down.saturating_sub(g.last_net_total_down)) as f64 * 8.0 / elapsed_secs) as u64;
            (up, dn)
        };
        g.last_net_total_up = total_up;
        g.last_net_total_down = total_down;
        g.last_sample_at = now;

        // 连接数
        let (tcp_conns, udp_conns) = read_sockstat();

        // uptime + 进程数
        let uptime_secs = System::uptime();
        let process_count = g.sys.processes().len() as u32;

        let s = g.static_info.clone();
        NodeMetrics {
            cpu_name: s.cpu_name,
            cpu_cores: s.cpu_cores,
            arch: s.arch,
            os: s.os,
            kernel: s.kernel,
            virtualization: s.virtualization,
            cpu_usage,
            ram_total,
            ram_used,
            swap_total,
            swap_used,
            disk_total,
            disk_used,
            load1: load.one,
            load5: load.five,
            load15: load.fifteen,
            net_up_bps: up_bps,
            net_down_bps: down_bps,
            net_total_up: total_up,
            net_total_down: total_down,
            tcp_conns,
            udp_conns,
            uptime_secs,
            process_count,
        }
    }
}

fn compute_static_info(sys: &System) -> StaticInfo {
    let cpus = sys.cpus();
    let cpu_name = cpus
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let cpu_cores = cpus.len() as u32;
    StaticInfo {
        cpu_name,
        cpu_cores,
        arch: System::cpu_arch().unwrap_or_default(),
        os: System::long_os_version().unwrap_or_else(|| System::name().unwrap_or_default()),
        kernel: System::kernel_version().unwrap_or_default(),
        virtualization: detect_virtualization(),
    }
}

/// 检测虚拟化类型。优先 systemd-detect-virt（命中即用），其次读 /sys/class/dmi。
fn detect_virtualization() -> String {
    if let Ok(out) = std::process::Command::new("systemd-detect-virt").output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            let t = s.trim();
            if !t.is_empty() && t != "none" {
                return t.to_string();
            } else if t == "none" {
                return "none".to_string();
            }
        }
    }
    // fallback
    if std::fs::read_to_string("/sys/class/dmi/id/product_name")
        .map(|s| s.to_lowercase().contains("kvm") || s.to_lowercase().contains("vmware"))
        .unwrap_or(false)
    {
        return "kvm-or-vmware".to_string();
    }
    "unknown".to_string()
}

/// /proc/net/sockstat 解析 TCP / UDP 当前打开 socket 数。
/// 文件结构例：
///   sockets: used 312
///   TCP: inuse 8 orphan 0 tw 1 alloc 13 mem 1
///   UDP: inuse 7 mem 2
/// 非 Linux 返回 (0, 0)。
fn read_sockstat() -> (u32, u32) {
    let content = match std::fs::read_to_string("/proc/net/sockstat") {
        Ok(s) => s,
        Err(_) => return (0, 0),
    };
    let mut tcp = 0u32;
    let mut udp = 0u32;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("TCP: inuse ") {
            tcp = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("UDP: inuse ") {
            udp = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }
    (tcp, udp)
}
