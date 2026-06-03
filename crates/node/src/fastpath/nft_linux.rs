//! Linux nftables 实现 — shell out 到 `nft` 命令。
//! 控制面操作（rule add/delete/list）不是 hot path，subprocess 开销可忽略；
//! 数据面（实际转发）100% 在内核 netfilter，0 syscall 0 user-space copy。
//!
//! table 设计：
//!   table inet iris {
//!     chain prerouting { type nat hook prerouting priority dstnat; ... }
//!     chain postrouting { type nat hook postrouting priority srcnat; masquerade }
//!   }
//!
//! 每条 forward 规则:
//!   add rule inet iris prerouting <tcp|udp> dport <P> counter dnat ip to <IP>:<PORT> \
//!     comment "iris-fwd-<ID>"
//!
//! 删除：list -a 拿 handle，delete by handle。
//!
//! 流量统计（M4.3）：
//!   nft prerouting counter 仅命中 conntrack 未建状态的首包 — 准确度 < 0.01%。
//!   改用 `/proc/net/nf_conntrack` (要求 sysctl `net.netfilter.nf_conntrack_acct=1`)，
//!   每条 entry 双向 bytes/packets 都准确，按 original-dport == listen_port 归属 forward。
//!
//!   节点端做 delta tracking：上次 tick 见过的 5-tuple flow 记录 last_seen 字节数，
//!   本 tick 见到同 flow 加增量。flow 结束（不再出现在 /proc）→ 从 last_seen 丢，
//!   累计 bytes 留在 cumulative。cumulative 单调递增（agent 重启回 0，master 端
//!   delta 算法识别为 epoch 重置）。

use super::{CounterSnapshot, FastPathManager, FastPathRule};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;

const TABLE: &str = "iris";
const CONNTRACK_PATH: &str = "/proc/net/nf_conntrack";
const CONNTRACK_ACCT_SYSCTL: &str = "/proc/sys/net/netfilter/nf_conntrack_acct";

#[derive(Default)]
pub struct NftFastPath {
    initialized: std::sync::atomic::AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// listen_port → forward_id。add/delete_rule 时更新；get_counters 时按
    /// conntrack entry original-dport 反查 forward_id。
    port_to_fid: HashMap<u16, i64>,
    /// 上次 tick 每条 flow 见到的 (original_bytes, reply_bytes)。
    /// key = "proto:src_ip:sport:dport" 唯一标识连接。
    /// flow 关闭（不再出现）→ 自动从 map 丢弃。
    flow_last: HashMap<String, (u64, u64)>,
    /// 每条 forward 自 agent 启动以来累计字节。单调递增，重启回 0。
    cumulative: HashMap<i64, (u64, u64)>,
    /// agent 启动后第一次 poll 是否已完成。
    /// 重要：节点重启时 kernel conntrack 仍保留 ESTABLISHED / TIME_WAIT 旧 flow（每条带历史字节数）。
    /// 不做 bootstrap 的话，首次 poll 把这些 ghost flow 的整段字节当 delta 加进 cumulative，
    /// 严重过计。第一次 poll 仅 populate flow_last 当 baseline，cumulative 不动 — 后续
    /// poll 从这个 baseline 算增量。master delta 算法在 cumulative<last 时识别为 epoch reset，
    /// 自然吃下"启动后看到的 delta 较小"这种情况。
    bootstrapped: bool,
}

impl NftFastPath {
    /// 用 stdin 跑 `nft -f -` 提交一个事务。
    fn nft_exec(input: &str) -> Result<()> {
        let mut child = Command::new("nft")
            .arg("-f")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("spawn nft: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "nft -f failed (exit {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }

    /// nft -a list chain ... → 抓出注释里有 "iris-fwd-<id>" 的行的 handle。
    fn find_rule_handles(forward_id: i64) -> Result<Vec<u64>> {
        let out = Command::new("nft")
            .args(["-a", "list", "chain", "inet", TABLE, "prerouting"])
            .output()
            .map_err(|e| anyhow!("spawn nft list: {e}"))?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let needle = format!("\"iris-fwd-{forward_id}\"");
        let mut handles = Vec::new();
        for line in text.lines() {
            if line.contains(&needle) {
                if let Some(h) = line.rsplit("# handle ").next() {
                    if let Ok(num) = h.trim().parse::<u64>() {
                        handles.push(num);
                    }
                }
            }
        }
        Ok(handles)
    }
}

impl FastPathManager for NftFastPath {
    fn init(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        // DNAT 把包送到外部目标必须 ip_forward=1，否则 kernel 不路由出去。
        // Ubuntu/Debian 默认是 0（仅做客户端使用）— 静默启用。
        for sysctl_path in [
            "/proc/sys/net/ipv4/ip_forward",
            "/proc/sys/net/ipv4/conf/all/forwarding",
        ] {
            if let Err(e) = std::fs::write(sysctl_path, "1\n") {
                tracing::warn!(path = %sysctl_path, error = %e, "enable ip_forward failed (DNAT may not route)");
            }
        }
        // M4.3 conntrack-acct：开启 kernel 在每条 conntrack entry 上记录 packets/bytes。
        // 默认在大多数现代发行版（Ubuntu 22.04+）= 1，但保守显式启用。失败仅 warn —
        // 失败时 /proc/net/nf_conntrack 的 bytes= 字段会一直是 0，fast forward 流量统计将报 0。
        if let Err(e) = std::fs::write(CONNTRACK_ACCT_SYSCTL, "1\n") {
            tracing::warn!(path = CONNTRACK_ACCT_SYSCTL, error = %e,
                "enable nf_conntrack_acct failed (fast path traffic stats will be 0)");
        }
        // 先清旧残留
        let _ = Self::nft_exec(&format!("delete table inet {TABLE}"));
        let script = format!(
            "table inet {TABLE} {{\n\
             \tchain prerouting {{ type nat hook prerouting priority dstnat; }}\n\
             \tchain postrouting {{ type nat hook postrouting priority srcnat; }}\n\
             }}\n\
             add rule inet {TABLE} postrouting masquerade comment \"iris-masq\"\n"
        );
        Self::nft_exec(&script)?;
        self.initialized.store(true, Ordering::Release);
        tracing::info!(table = TABLE, "fast path table installed");
        Ok(())
    }

    fn cleanup(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        let _ = Self::nft_exec(&format!("delete table inet {TABLE}"));
        self.initialized.store(false, Ordering::Release);
        if let Ok(mut st) = self.state.lock() {
            st.port_to_fid.clear();
            st.flow_last.clear();
            st.cumulative.clear();
        }
        Ok(())
    }

    fn add_rule(&self, rule: &FastPathRule) -> Result<()> {
        let proto = match rule.protocol.as_str() {
            "tcp" | "udp" => rule.protocol.as_str(),
            other => return Err(anyhow!("fastpath unsupported protocol: {other}")),
        };
        let ip = rule.target_addr.ip();
        let port = rule.target_addr.port();
        let (family_qual, ip_str) = match ip {
            std::net::IpAddr::V4(v4) => ("ip", v4.to_string()),
            std::net::IpAddr::V6(v6) => ("ip6", format!("[{v6}]")),
        };
        let script = format!(
            "add rule inet {TABLE} prerouting {proto} dport {} counter dnat {family_qual} to {ip_str}:{} \
             comment \"iris-fwd-{}\"\n",
            rule.listen_port, port, rule.forward_id
        );
        Self::nft_exec(&script)?;
        // 注册 listen_port → forward_id 映射（M4.3 用于 conntrack 归属）
        if let Ok(mut st) = self.state.lock() {
            st.port_to_fid.insert(rule.listen_port, rule.forward_id);
            // 新加 forward 预置 0 累计，让首次 heartbeat 就有 entry
            st.cumulative.entry(rule.forward_id).or_insert((0, 0));
        }
        Ok(())
    }

    fn delete_rule(&self, forward_id: i64) -> Result<()> {
        let handles = Self::find_rule_handles(forward_id)?;
        if !handles.is_empty() {
            let mut script = String::new();
            for h in handles {
                script.push_str(&format!(
                    "delete rule inet {TABLE} prerouting handle {h}\n"
                ));
            }
            Self::nft_exec(&script)?;
        }
        // 清状态：port_to_fid 反查删，cumulative 整条删（forward 没了）。
        // flow_last 不主动清 — 其中残留的 flow 自然会在下次 poll 时因为 port_to_fid
        // 找不到 fid 而忽略，下下次 poll 干脆不出现就被 swap 掉。
        if let Ok(mut st) = self.state.lock() {
            st.port_to_fid.retain(|_, fid| *fid != forward_id);
            st.cumulative.remove(&forward_id);
        }
        Ok(())
    }

    fn get_counters(&self) -> Result<HashMap<i64, CounterSnapshot>> {
        let text = match std::fs::read_to_string(CONNTRACK_PATH) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(path = CONNTRACK_PATH, error = %e, "read conntrack failed");
                // 读失败 → 返回当前 cumulative 不变（state 里的）
                return Ok(self.snapshot_cumulative());
            }
        };
        let mut st = self.state.lock().map_err(|_| anyhow!("fastpath state mutex poisoned"))?;
        let bootstrapping = !st.bootstrapped;
        let mut new_flow_last: HashMap<String, (u64, u64)> = HashMap::with_capacity(st.flow_last.len());
        for line in text.lines() {
            let Some(entry) = parse_conntrack_line(line) else { continue };
            let Some(&fid) = st.port_to_fid.get(&entry.dport) else { continue };
            let key = entry.flow_key();
            if bootstrapping {
                // 首次 poll：当前 conntrack 表里的 flow 字节算 baseline，不累加 — 避免
                // 把节点重启前的 ghost flow（TIME_WAIT 等）整段字节误计为新增。
                new_flow_last.insert(key, (entry.orig_bytes, entry.reply_bytes));
                continue;
            }
            let (last_in, last_out) = st.flow_last.get(&key).copied().unwrap_or((0, 0));
            // saturating_sub：极少数情况 kernel 偶发回退（conntrack 重建）→ 报 0 增量，
            // 不报错也不双计。
            let delta_in = entry.orig_bytes.saturating_sub(last_in);
            let delta_out = entry.reply_bytes.saturating_sub(last_out);
            // M-3（review fix）：get_mut 替代 entry().or_insert() — 明确"forward 已被
            // delete_rule 清掉 → 不重新建 entry"。port_to_fid 已是一道防线，但 get_mut
            // 显式拒绝 ghost fid，意图更清晰也少一份隐式契约。
            if let Some(cum) = st.cumulative.get_mut(&fid) {
                cum.0 = cum.0.saturating_add(delta_in);
                cum.1 = cum.1.saturating_add(delta_out);
                new_flow_last.insert(key, (entry.orig_bytes, entry.reply_bytes));
            }
        }
        st.flow_last = new_flow_last;
        if bootstrapping {
            st.bootstrapped = true;
            tracing::debug!(flows = st.flow_last.len(), "fastpath bootstrap poll: baseline established");
        }
        Ok(st.cumulative
            .iter()
            .map(|(fid, (bi, bo))| {
                (*fid, CounterSnapshot {
                    forward_id: *fid,
                    bytes_in: *bi,
                    bytes_out: *bo,
                })
            })
            .collect())
    }

    fn is_available(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl NftFastPath {
    fn snapshot_cumulative(&self) -> HashMap<i64, CounterSnapshot> {
        let st = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };
        st.cumulative
            .iter()
            .map(|(fid, (bi, bo))| {
                (*fid, CounterSnapshot {
                    forward_id: *fid,
                    bytes_in: *bi,
                    bytes_out: *bo,
                })
            })
            .collect()
    }
}

// ───── conntrack 行解析 ─────────────────────────────────────────

/// 一条 /proc/net/nf_conntrack 解析结果。仅 tcp/udp 才有意义。
struct CtEntry {
    proto: String, // "tcp" | "udp"
    src: String,   // 原始方向 src IP（客户端 IP）
    sport: u16,    // 原始方向 sport（客户端临时端口）
    dport: u16,    // 原始方向 dport（= forward listen_port）
    orig_bytes: u64,
    reply_bytes: u64,
}

impl CtEntry {
    fn flow_key(&self) -> String {
        format!("{}:{}:{}:{}", self.proto, self.src, self.sport, self.dport)
    }
}

/// 解析一行 conntrack。格式：
///   `ipv4 2 tcp 6 119 ESTABLISHED src=A dst=B sport=C dport=D packets=P bytes=BX src=B' dst=A' sport=D' dport=C' packets=P' bytes=BY [ASSURED] ...`
/// 第一对 src/sport/dport/packets/bytes = original direction（客户端→入口）
/// 第二对 = reply direction（入口→客户端）
/// 非 tcp/udp 返回 None。
fn parse_conntrack_line(line: &str) -> Option<CtEntry> {
    let mut proto: Option<String> = None;
    let mut src: Option<String> = None;
    let mut sport: Option<u16> = None;
    let mut dport: Option<u16> = None;
    let mut bytes_seen = 0u8;
    let mut orig_bytes = 0u64;
    let mut reply_bytes = 0u64;

    for tok in line.split_whitespace() {
        if (tok == "tcp" || tok == "udp") && proto.is_none() {
            proto = Some(tok.to_string());
        } else if let Some(v) = tok.strip_prefix("src=") {
            if src.is_none() {
                src = Some(v.to_string());
            }
        } else if let Some(v) = tok.strip_prefix("sport=") {
            if sport.is_none() {
                sport = v.parse().ok();
            }
        } else if let Some(v) = tok.strip_prefix("dport=") {
            if dport.is_none() {
                dport = v.parse().ok();
            }
        } else if let Some(v) = tok.strip_prefix("bytes=") {
            let val: u64 = v.parse().unwrap_or(0);
            // M-2（review fix）：saturating_add 防异常 conntrack 行含 >255 个 bytes= 键
            // 时 panic / wrap（实际不会发生，但 fail-safe）
            bytes_seen = bytes_seen.saturating_add(1);
            if bytes_seen == 1 {
                orig_bytes = val;
            } else if bytes_seen == 2 {
                reply_bytes = val;
            }
        }
    }

    Some(CtEntry {
        proto: proto?,
        src: src?,
        sport: sport?,
        dport: dport?,
        orig_bytes,
        reply_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_established() {
        let line = "ipv4     2 tcp      6 431999 ESTABLISHED src=10.146.0.6 dst=104.198.114.243 sport=43210 dport=9301 packets=1234 bytes=560000 src=23.149.108.114 dst=10.146.0.4 sport=5201 dport=43210 packets=987 bytes=12340000 [ASSURED] mark=0 use=2";
        let e = parse_conntrack_line(line).unwrap();
        assert_eq!(e.proto, "tcp");
        assert_eq!(e.src, "10.146.0.6");
        assert_eq!(e.sport, 43210);
        assert_eq!(e.dport, 9301);
        assert_eq!(e.orig_bytes, 560000);
        assert_eq!(e.reply_bytes, 12340000);
        assert_eq!(e.flow_key(), "tcp:10.146.0.6:43210:9301");
    }

    #[test]
    fn parse_udp_unreplied() {
        let line = "ipv4 2 udp 17 28 src=10.0.0.1 dst=10.0.0.2 sport=44444 dport=53 packets=1 bytes=64 [UNREPLIED] src=10.0.0.2 dst=10.0.0.1 sport=53 dport=44444 packets=0 bytes=0 mark=0 use=2";
        let e = parse_conntrack_line(line).unwrap();
        assert_eq!(e.proto, "udp");
        assert_eq!(e.dport, 53);
        assert_eq!(e.orig_bytes, 64);
        assert_eq!(e.reply_bytes, 0);
    }

    #[test]
    fn parse_icmp_skipped() {
        let line = "ipv4 2 icmp 1 29 src=1.2.3.4 dst=5.6.7.8 type=8 code=0 id=1234 packets=1 bytes=84 src=5.6.7.8 dst=1.2.3.4 type=0 code=0 id=1234 packets=1 bytes=84 mark=0 use=1";
        // icmp 没 sport/dport 关键字 → 解析失败 None
        assert!(parse_conntrack_line(line).is_none());
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_conntrack_line("").is_none());
        assert!(parse_conntrack_line("garbage line without keywords").is_none());
    }

    /// 模拟两次 poll 的 delta 累加。
    #[test]
    fn delta_tracking_simulates_correctly() {
        let fp = NftFastPath::default();
        fp.state.lock().unwrap().port_to_fid.insert(9301, 47);

        // 模拟首次 poll：flow A 已传 1000/2000
        {
            let mut st = fp.state.lock().unwrap();
            let key = "tcp:10.0.0.1:1234:9301".to_string();
            let (last_in, last_out) = (0, 0);
            let entry_orig = 1000u64;
            let entry_reply = 2000u64;
            let delta_in = entry_orig.saturating_sub(last_in);
            let delta_out = entry_reply.saturating_sub(last_out);
            let cum = st.cumulative.entry(47).or_insert((0, 0));
            cum.0 += delta_in;
            cum.1 += delta_out;
            st.flow_last.insert(key, (entry_orig, entry_reply));
        }

        // 第二次 poll：flow A 涨到 1500/3000
        {
            let mut st = fp.state.lock().unwrap();
            let key = "tcp:10.0.0.1:1234:9301".to_string();
            let (last_in, last_out) = st.flow_last.get(&key).copied().unwrap();
            let entry_orig = 1500u64;
            let entry_reply = 3000u64;
            let delta_in = entry_orig.saturating_sub(last_in);
            let delta_out = entry_reply.saturating_sub(last_out);
            let cum = st.cumulative.entry(47).or_insert((0, 0));
            cum.0 += delta_in;
            cum.1 += delta_out;
            st.flow_last.insert(key, (entry_orig, entry_reply));
        }

        let st = fp.state.lock().unwrap();
        let (bi, bo) = st.cumulative[&47];
        assert_eq!(bi, 1500); // 1000 + 500
        assert_eq!(bo, 3000); // 2000 + 1000
    }
}
