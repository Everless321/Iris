// M9 公开状态首页（komari 风格）—— 无需登录，作为对外门面展示节点健康状态。
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Card, Progress, Tag, Typography, Tooltip, Empty, Skeleton, theme } from "antd";
import { useAuth } from "../lib/auth";
import {
  CheckCircleFilled, MinusCircleFilled,
  CloudOutlined, DesktopOutlined, ArrowUpOutlined, ArrowDownOutlined,
} from "@ant-design/icons";

const { Title, Text } = Typography;

type Metrics = {
  cpu_name: string; cpu_cores: number; arch: string; os: string; kernel: string; virtualization: string;
  cpu_usage: number;
  ram_total: number; ram_used: number;
  swap_total: number; swap_used: number;
  disk_total: number; disk_used: number;
  load1: number; load5: number; load15: number;
  net_up_bps: number; net_down_bps: number;
  net_total_up: number; net_total_down: number;
  uptime_secs: number;
  updated_at: number;
} | null;

type NodeStatus = {
  id: string;
  name: string;
  online: boolean;
  last_seen: number | null;
  version: string;
  metrics: Metrics;
};

type StatusResp = {
  master_version: string;
  now: number;
  nodes: NodeStatus[];
};

function fmtBytes(n: number): string {
  if (!n || n <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(2) : v.toFixed(1)} ${units[i]}`;
}

function fmtBps(n: number): string {
  if (!n || n <= 0) return "0 B/s";
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

function fmtUptime(secs: number): string {
  if (!secs || secs <= 0) return "—";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function progressColor(pct: number): string {
  if (pct >= 90) return "#ff4d4f";
  if (pct >= 70) return "#faad14";
  return "#52c41a";
}

function StatusPill({ online }: { online: boolean }) {
  return online
    ? <Tag icon={<CheckCircleFilled />} color="success" style={{ margin: 0 }}>在线</Tag>
    : <Tag icon={<MinusCircleFilled />} color="error" style={{ margin: 0 }}>离线</Tag>;
}

function NodeCard({ node }: { node: NodeStatus }) {
  const m = node.metrics;
  const { token } = theme.useToken();
  const ramPct = m && m.ram_total > 0 ? (m.ram_used / m.ram_total) * 100 : 0;
  const diskPct = m && m.disk_total > 0 ? (m.disk_used / m.disk_total) * 100 : 0;
  const swapPct = m && m.swap_total > 0 ? (m.swap_used / m.swap_total) * 100 : 0;
  const cpuPct = m ? Math.min(100, Math.max(0, m.cpu_usage)) : 0;

  return (
    <Card
      style={{ borderRadius: 14, boxShadow: "0 1px 3px rgba(0,0,0,0.04)" }}
      bodyStyle={{ padding: 18 }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", marginBottom: 10 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 16, fontWeight: 600, marginBottom: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {node.name || node.id}
          </div>
          <Text type="secondary" style={{ fontSize: 12, fontFamily: "monospace" }}>{node.id}</Text>
        </div>
        <StatusPill online={node.online} />
      </div>

      {!m ? (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="无监控数据" style={{ margin: "20px 0" }} />
      ) : (
        <>
          {/* System info row */}
          <div style={{ display: "flex", gap: 12, marginBottom: 14, fontSize: 12, color: token.colorTextSecondary, flexWrap: "wrap" }}>
            <span><DesktopOutlined /> {m.os || m.arch || "—"}</span>
            {m.virtualization && m.virtualization !== "none" && <span><CloudOutlined /> {m.virtualization}</span>}
            <span>{m.cpu_cores} 核</span>
            <span>up {fmtUptime(m.uptime_secs)}</span>
          </div>

          {/* CPU */}
          <div style={{ marginBottom: 10 }}>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12, marginBottom: 4 }}>
              <Text type="secondary">CPU</Text>
              <Text style={{ fontFamily: "monospace" }}>{cpuPct.toFixed(1)}%</Text>
            </div>
            <Progress percent={cpuPct} strokeColor={progressColor(cpuPct)} showInfo={false} size="small" />
          </div>

          {/* Memory */}
          <div style={{ marginBottom: 10 }}>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12, marginBottom: 4 }}>
              <Text type="secondary">内存</Text>
              <Text style={{ fontFamily: "monospace" }}>{fmtBytes(m.ram_used)} / {fmtBytes(m.ram_total)}</Text>
            </div>
            <Progress percent={ramPct} strokeColor={progressColor(ramPct)} showInfo={false} size="small" />
          </div>

          {/* Disk */}
          <div style={{ marginBottom: 10 }}>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12, marginBottom: 4 }}>
              <Text type="secondary">磁盘</Text>
              <Text style={{ fontFamily: "monospace" }}>{fmtBytes(m.disk_used)} / {fmtBytes(m.disk_total)}</Text>
            </div>
            <Progress percent={diskPct} strokeColor={progressColor(diskPct)} showInfo={false} size="small" />
          </div>

          {/* Swap (only when configured) */}
          {m.swap_total > 0 && (
            <div style={{ marginBottom: 10 }}>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12, marginBottom: 4 }}>
                <Text type="secondary">Swap</Text>
                <Text style={{ fontFamily: "monospace" }}>{fmtBytes(m.swap_used)} / {fmtBytes(m.swap_total)}</Text>
              </div>
              <Progress percent={swapPct} strokeColor={progressColor(swapPct)} showInfo={false} size="small" />
            </div>
          )}

          {/* Net + Load footer */}
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: 12, paddingTop: 12, borderTop: `1px solid ${token.colorBorderSecondary}`, fontSize: 12, fontFamily: "monospace" }}>
            <Tooltip title={`累计上传 ${fmtBytes(m.net_total_up)}`}>
              <span><ArrowUpOutlined style={{ color: "#1677ff" }} /> {fmtBps(m.net_up_bps)}</span>
            </Tooltip>
            <Tooltip title={`累计下载 ${fmtBytes(m.net_total_down)}`}>
              <span><ArrowDownOutlined style={{ color: "#52c41a" }} /> {fmtBps(m.net_down_bps)}</span>
            </Tooltip>
            <Tooltip title={`load 1m / 5m / 15m`}>
              <span style={{ color: token.colorTextSecondary }}>
                {m.load1.toFixed(2)} {m.load5.toFixed(2)} {m.load15.toFixed(2)}
              </span>
            </Tooltip>
          </div>
        </>
      )}
    </Card>
  );
}

export default function StatusBoard() {
  const user = useAuth((s) => s.user);
  const [data, setData] = useState<StatusResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let aborted = false;
    const load = async () => {
      try {
        const r = await fetch("/api/public/status");
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const j: StatusResp = await r.json();
        if (!aborted) {
          setData(j);
          setErr(null);
        }
      } catch (e) {
        if (!aborted) setErr((e as Error).message);
      } finally {
        if (!aborted) setLoading(false);
      }
    };
    load();
    const t = setInterval(load, 5000);
    return () => { aborted = true; clearInterval(t); };
  }, []);

  const summary = useMemo(() => {
    if (!data) return { total: 0, online: 0 };
    const total = data.nodes.length;
    const online = data.nodes.filter((n) => n.online).length;
    return { total, online };
  }, [data]);

  return (
    <div style={{ minHeight: "100dvh", background: "#fafbfc" }}>
      <div style={{ maxWidth: 1320, margin: "0 auto", padding: "32px 24px 48px" }}>
        {/* Header */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", marginBottom: 28, flexWrap: "wrap", gap: 16 }}>
          <div>
            <Title level={2} style={{ margin: 0, fontWeight: 600, letterSpacing: -0.5 }}>
              Iris · 节点状态
            </Title>
            <Text type="secondary" style={{ fontSize: 14 }}>
              {summary.online}/{summary.total} 在线 · 5 秒刷新
            </Text>
          </div>
          <Link to={user ? "/admin" : "/login"} style={{ color: "#1677ff", fontSize: 13 }}>
            {user ? "进入控制台" : "管理员登录"} →
          </Link>
        </div>

        {/* Grid */}
        {loading ? (
          <div style={{ display: "grid", gap: 16, gridTemplateColumns: "repeat(auto-fill, minmax(320px, 1fr))" }}>
            {[1, 2, 3].map((i) => (
              <Card key={i} style={{ borderRadius: 14 }}><Skeleton active /></Card>
            ))}
          </div>
        ) : err ? (
          <Empty description={`加载失败：${err}`} />
        ) : !data || data.nodes.length === 0 ? (
          <Empty description="尚无节点" />
        ) : (
          <div style={{ display: "grid", gap: 16, gridTemplateColumns: "repeat(auto-fill, minmax(320px, 1fr))" }}>
            {data.nodes.map((n) => <NodeCard key={n.id} node={n} />)}
          </div>
        )}

        {data && (
          <div style={{ marginTop: 32, textAlign: "center", fontSize: 12, color: "#999" }}>
            master {data.master_version} · powered by Iris
          </div>
        )}
      </div>
    </div>
  );
}
