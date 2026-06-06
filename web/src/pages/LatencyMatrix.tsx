import { useEffect, useMemo, useState } from "react";
import { Card, Typography, Skeleton, Empty, Space, Tag, Button, Alert } from "antd";
import { ReloadOutlined, NodeIndexOutlined } from "@ant-design/icons";
import { api } from "../lib/api";

const { Title, Text } = Typography;

type NodeRef = { id: string; name: string };
type MatrixResp = {
  nodes: NodeRef[];
  matrix: Record<string, Record<string, number>>;
  ttl_secs: number;
};

function rttColor(ms: number): string {
  if (ms < 30) return "#52c41a";
  if (ms < 80) return "#a0d911";
  if (ms < 150) return "#faad14";
  if (ms < 300) return "#fa8c16";
  return "#f5222d";
}

function rttBg(ms: number): string {
  if (ms < 30) return "rgba(82,196,26,0.12)";
  if (ms < 80) return "rgba(160,217,17,0.12)";
  if (ms < 150) return "rgba(250,173,20,0.14)";
  if (ms < 300) return "rgba(250,140,22,0.16)";
  return "rgba(245,34,45,0.18)";
}

export default function LatencyMatrix() {
  const [data, setData] = useState<MatrixResp | null>(null);
  const [err, setErr] = useState<string>("");
  const [loading, setLoading] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      const r = await api.get<MatrixResp>("/api/latency-matrix");
      setData(r);
      setErr("");
    } catch (e) {
      setErr((e as Error)?.message || "加载失败");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  const stats = useMemo(() => {
    if (!data) return { edges: 0, avg: 0, min: 0, max: 0 };
    const vals: number[] = [];
    for (const row of Object.values(data.matrix)) {
      for (const v of Object.values(row)) vals.push(v);
    }
    if (vals.length === 0) return { edges: 0, avg: 0, min: 0, max: 0 };
    const sum = vals.reduce((a, b) => a + b, 0);
    return {
      edges: vals.length,
      avg: Math.round(sum / vals.length),
      min: Math.min(...vals),
      max: Math.max(...vals),
    };
  }, [data]);

  if (!data && loading) {
    return (
      <div style={{ maxWidth: 1400, margin: "0 auto" }}>
        <Skeleton active />
      </div>
    );
  }

  const nodes = data?.nodes ?? [];
  const matrix = data?.matrix ?? {};

  return (
    <div style={{ maxWidth: 1400, margin: "0 auto" }}>
      <div style={{ marginBottom: 16, display: "flex", alignItems: "flex-end", justifyContent: "space-between" }}>
        <div>
          <Title level={3} style={{ marginBottom: 4 }}>
            <NodeIndexOutlined /> 节点延迟矩阵
          </Title>
          <Text type="secondary">
            每个节点对邻居 TCP 探测的真实 RTT（EWMA 平滑，5s 一轮）。
            行 = 入口节点视角；列 = 目标邻居。空格 = 该方向暂无样本或已过 TTL。
          </Text>
        </div>
        <Space>
          <Tag>TTL {data?.ttl_secs ?? 60}s</Tag>
          <Tag>样本 {stats.edges}</Tag>
          {stats.edges > 0 && (
            <>
              <Tag color="blue">min {stats.min}ms</Tag>
              <Tag color="geekblue">avg {stats.avg}ms</Tag>
              <Tag color="volcano">max {stats.max}ms</Tag>
            </>
          )}
          <Button icon={<ReloadOutlined />} loading={loading} onClick={load}>
            刷新
          </Button>
        </Space>
      </div>

      {err && <Alert type="error" message={err} style={{ marginBottom: 12 }} showIcon />}

      <Card bodyStyle={{ padding: 0 }}>
        {nodes.length === 0 ? (
          <Empty description="尚无节点" style={{ padding: 48 }} />
        ) : stats.edges === 0 ? (
          <Empty description="尚无邻居延迟样本（节点首次启动后约 5–10s 出现）" style={{ padding: 48 }} />
        ) : (
          <div style={{ overflowX: "auto" }}>
            <table style={{ borderCollapse: "collapse", width: "100%", fontVariantNumeric: "tabular-nums" }}>
              <thead>
                <tr>
                  <th style={cellHeadCorner}>from \ to</th>
                  {nodes.map((n) => (
                    <th key={n.id} style={cellHead} title={n.id}>
                      {n.name || n.id}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {nodes.map((from) => (
                  <tr key={from.id}>
                    <th style={cellHeadRow} title={from.id}>
                      {from.name || from.id}
                    </th>
                    {nodes.map((to) => {
                      if (from.id === to.id) {
                        return <td key={to.id} style={{ ...cell, background: "#fafafa", color: "#bfbfbf" }}>—</td>;
                      }
                      const rtt = matrix[from.id]?.[to.id];
                      if (rtt === undefined) {
                        return <td key={to.id} style={{ ...cell, color: "#bfbfbf" }}>·</td>;
                      }
                      return (
                        <td
                          key={to.id}
                          style={{
                            ...cell,
                            background: rttBg(rtt),
                            color: rttColor(rtt),
                            fontWeight: 600,
                          }}
                          title={`${from.name || from.id} → ${to.name || to.id}: ${rtt}ms`}
                        >
                          {rtt}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      <div style={{ marginTop: 12, color: "#999", fontSize: 12 }}>
        说明：数字单位 ms。空白格 = 该 from→to 方向暂无样本（节点未启动探测、连续失败被剔除、或超过 {data?.ttl_secs ?? 60}s TTL）。
        矩阵不对称是正常现象（双向网络路径常常不同）。
      </div>
    </div>
  );
}

const cell: React.CSSProperties = {
  border: "1px solid #f0f0f0",
  padding: "8px 12px",
  textAlign: "center",
  minWidth: 64,
};

const cellHead: React.CSSProperties = {
  ...cell,
  background: "#fafafa",
  fontWeight: 600,
  whiteSpace: "nowrap",
};

const cellHeadRow: React.CSSProperties = {
  ...cellHead,
  textAlign: "left",
  position: "sticky",
  left: 0,
  zIndex: 1,
};

const cellHeadCorner: React.CSSProperties = {
  ...cellHead,
  textAlign: "left",
  position: "sticky",
  left: 0,
  zIndex: 2,
  color: "#999",
  fontWeight: 500,
};
