import { useEffect, useState } from "react";
import { Modal, Row, Col, Statistic, Progress, Card, Tag, Empty, Skeleton, Typography } from "antd";
import { api } from "../lib/api";

const { Text } = Typography;

type Metrics = {
  node_id: string;
  cpu_name: string; cpu_cores: number; arch: string;
  os: string; kernel: string; virtualization: string;
  cpu_usage: number;
  ram_total: number; ram_used: number;
  swap_total: number; swap_used: number;
  disk_total: number; disk_used: number;
  load1: number; load5: number; load15: number;
  net_up_bps: number; net_down_bps: number;
  net_total_up: number; net_total_down: number;
  tcp_conns: number; udp_conns: number;
  uptime_secs: number; process_count: number;
  updated_at: number;
};

type History = {
  ts_ms: number;
  cpu_usage: number;
  ram_used: number;
  net_up_bps: number;
  net_down_bps: number;
};

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  if (n < 1024 ** 4) return `${(n / 1024 ** 3).toFixed(2)} GB`;
  return `${(n / 1024 ** 4).toFixed(2)} TB`;
}

function fmtBps(bps: number): string {
  if (bps < 1000) return `${bps} bps`;
  if (bps < 1_000_000) return `${(bps / 1000).toFixed(1)} Kbps`;
  if (bps < 1_000_000_000) return `${(bps / 1_000_000).toFixed(1)} Mbps`;
  return `${(bps / 1_000_000_000).toFixed(2)} Gbps`;
}

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60), s = secs % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60), mr = m % 60;
  if (h < 24) return `${h}h ${mr}m`;
  const d = Math.floor(h / 24), hr = h % 24;
  return `${d}d ${hr}h`;
}

function gaugeColor(pct: number): string {
  if (pct >= 90) return "#ff4d4f";
  if (pct >= 75) return "#faad14";
  return "#52c41a";
}

// 简单 SVG 双轴折线图。x = 时间窗（最近 1h），y1 = cpu_usage %，y2 = net_up/down Mbps
function HistoryChart({ data }: { data: History[] }) {
  if (data.length < 2) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="历史数据不足" />;
  }
  const W = 720, H = 200, PAD_L = 36, PAD_R = 36, PAD_T = 10, PAD_B = 22;
  const innerW = W - PAD_L - PAD_R, innerH = H - PAD_T - PAD_B;
  const xs = data.map((d) => d.ts_ms);
  const xMin = xs[0], xMax = xs[xs.length - 1];
  const xScale = (t: number) => PAD_L + ((t - xMin) / Math.max(1, xMax - xMin)) * innerW;

  const cpuYScale = (v: number) => PAD_T + innerH - (v / 100) * innerH;
  const cpuPath = data
    .map((d, i) => `${i === 0 ? "M" : "L"} ${xScale(d.ts_ms).toFixed(1)} ${cpuYScale(d.cpu_usage).toFixed(1)}`)
    .join(" ");

  const netMax = Math.max(1, ...data.flatMap((d) => [d.net_up_bps, d.net_down_bps])) / 1_000_000; // Mbps
  const netYScale = (vBps: number) => PAD_T + innerH - (vBps / 1_000_000 / netMax) * innerH;
  const upPath = data.map((d, i) => `${i === 0 ? "M" : "L"} ${xScale(d.ts_ms).toFixed(1)} ${netYScale(d.net_up_bps).toFixed(1)}`).join(" ");
  const dnPath = data.map((d, i) => `${i === 0 ? "M" : "L"} ${xScale(d.ts_ms).toFixed(1)} ${netYScale(d.net_down_bps).toFixed(1)}`).join(" ");

  // x 轴标签：4 等分
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((p) => xMin + (xMax - xMin) * p);

  return (
    <div>
      <svg width={W} height={H} style={{ background: "#fafafa", borderRadius: 6, width: "100%" }} viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        {/* 网格 */}
        {[0, 0.25, 0.5, 0.75, 1].map((p) => (
          <line key={p} x1={PAD_L} x2={W - PAD_R} y1={PAD_T + innerH * p} y2={PAD_T + innerH * p}
            stroke="#e8e8e8" strokeDasharray="3,3" />
        ))}
        {/* CPU usage */}
        <path d={cpuPath} fill="none" stroke="#1677ff" strokeWidth={1.5} />
        {/* 上下行 */}
        <path d={upPath} fill="none" stroke="#52c41a" strokeWidth={1.5} />
        <path d={dnPath} fill="none" stroke="#722ed1" strokeWidth={1.5} />
        {/* y 轴左：CPU 0-100 */}
        {[0, 50, 100].map((v) => (
          <text key={v} x={PAD_L - 4} y={cpuYScale(v) + 3} fontSize={10} fill="#999" textAnchor="end">{v}</text>
        ))}
        {/* y 轴右：Net Mbps */}
        {[0, netMax / 2, netMax].map((v) => (
          <text key={v} x={W - PAD_R + 4} y={netYScale(v * 1_000_000) + 3} fontSize={10} fill="#999">{v.toFixed(1)}M</text>
        ))}
        {/* x 轴标签 */}
        {ticks.map((t) => (
          <text key={t} x={xScale(t)} y={H - 6} fontSize={10} fill="#999" textAnchor="middle">
            {new Date(t).toLocaleTimeString().slice(0, 5)}
          </text>
        ))}
      </svg>
      <div style={{ display: "flex", gap: 16, fontSize: 12, color: "#666", marginTop: 6 }}>
        <span><span style={{ display: "inline-block", width: 10, height: 2, background: "#1677ff", verticalAlign: "middle" }} /> CPU %</span>
        <span><span style={{ display: "inline-block", width: 10, height: 2, background: "#52c41a", verticalAlign: "middle" }} /> 上行</span>
        <span><span style={{ display: "inline-block", width: 10, height: 2, background: "#722ed1", verticalAlign: "middle" }} /> 下行</span>
      </div>
    </div>
  );
}

type Props = {
  nodeId: string | null;
  onClose: () => void;
};

export default function NodeMetricsModal({ nodeId, onClose }: Props) {
  const [latest, setLatest] = useState<Metrics | null>(null);
  const [history, setHistory] = useState<History[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!nodeId) return;
    let alive = true;
    const fetchAll = async () => {
      try {
        const [m, h] = await Promise.all([
          api.get<Metrics | null>(`/api/nodes/${nodeId}/metrics`),
          api.get<History[]>(`/api/nodes/${nodeId}/metrics/history?window=3600`),
        ]);
        if (!alive) return;
        setLatest(m);
        setHistory(h);
      } catch {
        if (alive) {
          setLatest(null);
          setHistory([]);
        }
      } finally {
        if (alive) setLoading(false);
      }
    };
    setLoading(true);
    fetchAll();
    const t = setInterval(fetchAll, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [nodeId]);

  const ramPct = latest && latest.ram_total > 0 ? (latest.ram_used / latest.ram_total) * 100 : 0;
  const diskPct = latest && latest.disk_total > 0 ? (latest.disk_used / latest.disk_total) * 100 : 0;
  const swapPct = latest && latest.swap_total > 0 ? (latest.swap_used / latest.swap_total) * 100 : 0;

  return (
    <Modal
      open={!!nodeId}
      onCancel={onClose}
      footer={null}
      width={800}
      title={nodeId ? `节点监控 — ${nodeId}` : ""}
      destroyOnClose
    >
      {loading && !latest ? (
        <Skeleton active />
      ) : !latest ? (
        <Empty description="该节点尚未上报监控数据（老版本 binary，请升级到 M7+）" />
      ) : (
        <>
          {/* 静态信息 */}
          <Card size="small" style={{ marginBottom: 12 }}>
            <Row gutter={[8, 4]}>
              <Col span={12}><Text type="secondary">CPU:</Text> <Text>{latest.cpu_name || "—"} × {latest.cpu_cores}</Text></Col>
              <Col span={12}><Text type="secondary">架构:</Text> <Text className="num">{latest.arch || "—"}</Text></Col>
              <Col span={12}><Text type="secondary">系统:</Text> <Text>{latest.os || "—"}</Text></Col>
              <Col span={12}><Text type="secondary">内核:</Text> <Text className="num">{latest.kernel || "—"}</Text></Col>
              <Col span={12}><Text type="secondary">虚拟化:</Text> <Tag>{latest.virtualization || "—"}</Tag></Col>
              <Col span={12}><Text type="secondary">在线:</Text> <Text>{fmtUptime(latest.uptime_secs)}</Text></Col>
            </Row>
          </Card>

          {/* 仪表盘 */}
          <Row gutter={[12, 12]} style={{ marginBottom: 12 }}>
            <Col xs={12} md={6}>
              <Card size="small">
                <div style={{ textAlign: "center" }}>
                  <Progress
                    type="dashboard"
                    percent={+latest.cpu_usage.toFixed(1)}
                    strokeColor={gaugeColor(latest.cpu_usage)}
                    size={100}
                  />
                  <div style={{ marginTop: 4, fontSize: 12, color: "#666" }}>CPU 使用率</div>
                </div>
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <div style={{ textAlign: "center" }}>
                  <Progress
                    type="dashboard"
                    percent={+ramPct.toFixed(1)}
                    strokeColor={gaugeColor(ramPct)}
                    size={100}
                  />
                  <div style={{ marginTop: 4, fontSize: 12, color: "#666" }}>
                    内存 {fmtBytes(latest.ram_used)} / {fmtBytes(latest.ram_total)}
                  </div>
                </div>
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <div style={{ textAlign: "center" }}>
                  <Progress
                    type="dashboard"
                    percent={+diskPct.toFixed(1)}
                    strokeColor={gaugeColor(diskPct)}
                    size={100}
                  />
                  <div style={{ marginTop: 4, fontSize: 12, color: "#666" }}>
                    磁盘 {fmtBytes(latest.disk_used)} / {fmtBytes(latest.disk_total)}
                  </div>
                </div>
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <div style={{ textAlign: "center" }}>
                  <Progress
                    type="dashboard"
                    percent={+swapPct.toFixed(1)}
                    strokeColor={gaugeColor(swapPct)}
                    size={100}
                  />
                  <div style={{ marginTop: 4, fontSize: 12, color: "#666" }}>
                    Swap {fmtBytes(latest.swap_used)} / {fmtBytes(latest.swap_total)}
                  </div>
                </div>
              </Card>
            </Col>
          </Row>

          {/* 数字卡片 */}
          <Row gutter={[12, 12]} style={{ marginBottom: 12 }}>
            <Col xs={12} md={6}>
              <Card size="small">
                <Statistic title="Load (1/5/15)"
                  value={`${latest.load1.toFixed(2)} / ${latest.load5.toFixed(2)} / ${latest.load15.toFixed(2)}`}
                  valueStyle={{ fontSize: 16 }}
                />
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <Statistic title="网速 ↑" value={fmtBps(latest.net_up_bps)} valueStyle={{ fontSize: 16, color: "#52c41a" }} />
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <Statistic title="网速 ↓" value={fmtBps(latest.net_down_bps)} valueStyle={{ fontSize: 16, color: "#722ed1" }} />
              </Card>
            </Col>
            <Col xs={12} md={6}>
              <Card size="small">
                <Statistic
                  title={`累计流量 ↑${fmtBytes(latest.net_total_up)} / ↓`}
                  value={fmtBytes(latest.net_total_down)}
                  valueStyle={{ fontSize: 16 }}
                />
              </Card>
            </Col>
          </Row>

          <Row gutter={[12, 12]} style={{ marginBottom: 12 }}>
            <Col xs={8}>
              <Card size="small">
                <Statistic title="TCP 连接" value={latest.tcp_conns} valueStyle={{ fontSize: 18 }} />
              </Card>
            </Col>
            <Col xs={8}>
              <Card size="small">
                <Statistic title="UDP socket" value={latest.udp_conns} valueStyle={{ fontSize: 18 }} />
              </Card>
            </Col>
            <Col xs={8}>
              <Card size="small">
                <Statistic title="进程数" value={latest.process_count} valueStyle={{ fontSize: 18 }} />
              </Card>
            </Col>
          </Row>

          {/* 1h 趋势图 */}
          <Card size="small" title="最近 1 小时趋势（CPU + 网速）">
            {history && history.length > 0 ? (
              <HistoryChart data={history} />
            ) : (
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="无历史数据" />
            )}
          </Card>

          <div style={{ marginTop: 8, fontSize: 11, color: "#999", textAlign: "right" }}>
            更新于 {new Date(latest.updated_at).toLocaleString()}
          </div>
        </>
      )}
    </Modal>
  );
}
