import { useEffect, useState } from "react";
import { Card, Row, Col, Statistic, Typography, Skeleton, Empty, Tag, Space } from "antd";
import {
  LineChart, Line, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid,
} from "recharts";
import {
  CheckCircleFilled, MinusCircleFilled, QuestionCircleFilled,
  CloudServerOutlined, AreaChartOutlined, AlertOutlined,
} from "@ant-design/icons";
import { api, type Sla } from "../lib/api";

const { Title, Text } = Typography;

type Point = { ts: number; latency_ms: number | null; ok: number };
type Samples = Record<string, Point[]>;

function HealthTag({ h }: { h: string }) {
  if (h === "healthy") return <Tag icon={<CheckCircleFilled />} color="success">在线</Tag>;
  if (h === "unhealthy") return <Tag icon={<MinusCircleFilled />} color="error">离线</Tag>;
  return <Tag icon={<QuestionCircleFilled />} color="default">未知</Tag>;
}

export default function SlaBoard() {
  const [sla, setSla] = useState<Sla | null>(null);
  const [samples, setSamples] = useState<Samples>({});

  useEffect(() => {
    const load = async () => {
      try {
        const s = await api.get<Sla>("/api/sla");
        setSla(s);
        const sm = await api.get<Samples>("/api/sla/samples");
        setSamples(sm);
      } catch {}
    };
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  if (!sla) {
    return (
      <div style={{ maxWidth: 1400, margin: "0 auto" }}>
        <Skeleton active />
      </div>
    );
  }

  const totalFails = sla.nodes.reduce((s, n) => s + n.fail_events, 0);
  const avgUptime =
    sla.nodes.length > 0
      ? (sla.nodes.reduce((s, n) => s + n.uptime, 0) / sla.nodes.length) * 100
      : 0;

  return (
    <div style={{ maxWidth: 1400, margin: "0 auto" }}>
      <div style={{ marginBottom: 24 }}>
        <Title level={3} style={{ marginBottom: 4 }}>SLA 看板</Title>
        <Text type="secondary">节点健康状态、可用率、延迟趋势</Text>
      </div>

      <Row gutter={[16, 16]} style={{ marginBottom: 24 }}>
        <Col xs={24} md={8}>
          <Card size="small">
            <Statistic
              title={<Space size={6}><CloudServerOutlined />在线节点</Space>}
              value={sla.online}
              suffix={`/ ${sla.total}`}
              valueStyle={{ color: sla.online === sla.total ? "#52c41a" : "#faad14" }}
            />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card size="small">
            <Statistic
              title={<Space size={6}><AreaChartOutlined />平均可用率</Space>}
              value={avgUptime}
              precision={2}
              suffix="%"
              valueStyle={{ color: "#1677ff" }}
            />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card size="small">
            <Statistic
              title={<Space size={6}><AlertOutlined />故障事件</Space>}
              value={totalFails}
              valueStyle={{ color: totalFails > 0 ? "#ff4d4f" : undefined }}
            />
          </Card>
        </Col>
      </Row>

      <Card title="节点延迟 · 近 1 小时">
        {sla.nodes.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有节点" />
        ) : (
          <Row gutter={[16, 16]}>
            {sla.nodes.map((n) => {
              const pts = (samples[n.id] || []).map((p) => ({
                t: new Date(p.ts).toLocaleTimeString().slice(0, 5),
                latency: p.ok ? p.latency_ms : null,
              }));
              return (
                <Col xs={24} lg={12} key={n.id}>
                  <Card size="small" hoverable>
                    <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
                      <div>
                        <Space>
                          <Text strong>{n.name}</Text>
                          <HealthTag h={n.health} />
                        </Space>
                        <div style={{ marginTop: 2 }}>
                          <Text className="num" type="secondary" style={{ fontSize: 12 }}>{n.id}</Text>
                        </div>
                      </div>
                      <div style={{ textAlign: "right" }}>
                        <div className="num" style={{ fontSize: 18, fontWeight: 600, color: "#1677ff" }}>
                          {(n.uptime * 100).toFixed(1)}%
                        </div>
                        <Text type="secondary" style={{ fontSize: 11 }}>可用率</Text>
                      </div>
                    </div>
                    <ResponsiveContainer width="100%" height={160}>
                      <LineChart data={pts} margin={{ top: 4, right: 4, left: -16, bottom: 0 }}>
                        <CartesianGrid stroke="#f0f0f0" strokeDasharray="3 3" vertical={false} />
                        <XAxis
                          dataKey="t" stroke="#bfbfbf"
                          tick={{ fontSize: 10, fontFamily: "JetBrains Mono, monospace" }}
                          tickLine={false} axisLine={false}
                          interval="preserveStartEnd"
                        />
                        <YAxis
                          stroke="#bfbfbf"
                          tick={{ fontSize: 10, fontFamily: "JetBrains Mono, monospace" }}
                          tickLine={false} axisLine={false} unit="ms" width={40}
                        />
                        <Tooltip
                          contentStyle={{ background: "#fff", border: "1px solid #f0f0f0", fontSize: 12, borderRadius: 6 }}
                          labelStyle={{ color: "#8c8c8c" }}
                          cursor={{ stroke: "#1677ff", strokeOpacity: 0.3 }}
                        />
                        <Line
                          type="monotone" dataKey="latency"
                          stroke="#1677ff" strokeWidth={2}
                          dot={false} connectNulls={false} isAnimationActive={false}
                        />
                      </LineChart>
                    </ResponsiveContainer>
                  </Card>
                </Col>
              );
            })}
          </Row>
        )}
      </Card>
    </div>
  );
}
