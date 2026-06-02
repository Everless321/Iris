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
//!   add rule inet iris prerouting <tcp|udp> dport <P> counter dnat to <IP>:<PORT> \
//!     comment "iris-fwd-<ID>"
//!
//! 删除：list -a 拿 handle，delete by handle。
//! 拉计数器：nft -j list table inet iris，解 JSON 找 comment 匹配的 rule.counter。

use super::{CounterSnapshot, FastPathManager, FastPathRule};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

const TABLE: &str = "iris";

#[derive(Default)]
pub struct NftFastPath {
    initialized: std::sync::atomic::AtomicBool,
}

impl NftFastPath {
    /// 用 stdin 跑 `nft -f -` 提交一个事务。返回 stderr 文本（成功为空）。
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
            // 表不存在等情形 → 无规则可删
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let needle = format!("\"iris-fwd-{forward_id}\"");
        let mut handles = Vec::new();
        for line in text.lines() {
            if line.contains(&needle) {
                // 形如: ... comment "iris-fwd-42" # handle 7
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
        // 先清旧残留（上一次 agent crash 留的），失败忽略
        let _ = Self::nft_exec(&format!("delete table inet {TABLE}"));
        // 新建表 + 两条 nat chain；postrouting 默认 masquerade
        // priority: dstnat=-100, srcnat=100（kernel 默认值，nft 关键字自动展开）
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
        Ok(())
    }

    fn add_rule(&self, rule: &FastPathRule) -> Result<()> {
        let proto = match rule.protocol.as_str() {
            "tcp" | "udp" => rule.protocol.as_str(),
            other => return Err(anyhow!("fastpath unsupported protocol: {other}")),
        };
        let ip = rule.target_addr.ip();
        let port = rule.target_addr.port();
        // inet 表混合 v4/v6，dnat 必须显式 ip/ip6 限定 family（否则报
        // "ip or ip6 must be specified with address for inet tables"）。
        let (family_qual, ip_str) = match ip {
            std::net::IpAddr::V4(v4) => ("ip", v4.to_string()),
            std::net::IpAddr::V6(v6) => ("ip6", format!("[{v6}]")),
        };
        let script = format!(
            "add rule inet {TABLE} prerouting {proto} dport {} counter dnat {family_qual} to {ip_str}:{} \
             comment \"iris-fwd-{}\"\n",
            rule.listen_port, port, rule.forward_id
        );
        Self::nft_exec(&script)
    }

    fn delete_rule(&self, forward_id: i64) -> Result<()> {
        let handles = Self::find_rule_handles(forward_id)?;
        if handles.is_empty() {
            return Ok(());
        }
        // 多个 handle 一起删（同 forward_id 可能有多条 — 防御性处理）
        let mut script = String::new();
        for h in handles {
            script.push_str(&format!(
                "delete rule inet {TABLE} prerouting handle {h}\n"
            ));
        }
        Self::nft_exec(&script)
    }

    fn get_counters(&self) -> Result<HashMap<i64, CounterSnapshot>> {
        let out = Command::new("nft")
            .args(["-j", "list", "table", "inet", TABLE])
            .output()
            .map_err(|e| anyhow!("spawn nft -j list: {e}"))?;
        if !out.status.success() {
            return Ok(HashMap::new());
        }
        let v: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| anyhow!("parse nft json: {e}"))?;
        let mut map: HashMap<i64, CounterSnapshot> = HashMap::new();
        // nft -j 顶层：{ "nftables": [{ "metainfo": {...} }, { "table": {...} }, { "chain": {...} }, { "rule": {...} }, ...] }
        let Some(items) = v.get("nftables").and_then(|x| x.as_array()) else {
            return Ok(map);
        };
        for item in items {
            let Some(rule) = item.get("rule") else { continue };
            let Some(comment) = rule.get("comment").and_then(|c| c.as_str()) else { continue };
            let Some(fid_str) = comment.strip_prefix("iris-fwd-") else { continue };
            let Ok(fid) = fid_str.parse::<i64>() else { continue };
            // expr 是 array of { "match": ... } / { "counter": ... } / { "dnat": ... }
            let Some(exprs) = rule.get("expr").and_then(|e| e.as_array()) else { continue };
            for e in exprs {
                if let Some(c) = e.get("counter") {
                    let bytes = c.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                    map.entry(fid).or_insert(CounterSnapshot {
                        forward_id: fid,
                        bytes_in: bytes,
                        bytes_out: 0, // V2 conntrack 反向
                    });
                    break;
                }
            }
        }
        Ok(map)
    }

    fn is_available(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::Acquire)
    }
}
