import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { Card, Row, Col, Statistic, List, Empty, Skeleton, Tag, Space, Button, Typography } from "antd";
import {
  SwapOutlined,
  DatabaseOutlined,
  AreaChartOutlined,
  AlertOutlined,
  PlusOutlined,
  ArrowRightOutlined,
} from "@ant-design/icons";
import { api, type Sla, type Forward } from "../lib/api";
import { useAuth } from "../lib/auth";

const { Title, Text } = Typography;

export default function Dashboard() {
  const { user } = useAuth();
  const [sla, setSla] = useState<Sla | null>(null);
  const [fws, setFws] = useState<Forward[] | null>(null);

  useEffect(() => {
    api.get<Forward[]>("/api/forwards").then(setFws).catch(() => setFws([]));
    if (user?.role === "admin") api.get<Sla>("/api/sla").then(setSla).catch(() => {});
  }, [user]);

  const totalFails = sla?.nodes.reduce((s, n) => s + n.fail_events, 0) ?? 0;
  const avgUptime =
    sla && sla.nodes.length
      ? (sla.nodes.reduce((s, n) => s + n.uptime, 0) / sla.nodes.length) * 100
      : null;

  return (
    <div style={{ maxWidth: 1280, margin: "0 auto" }}>
      <div style={{ marginBottom: 24 }}>
        <Title level={3} style={{ marginBottom: 4 }}>
          欢迎回来，{user?.username}
        </Title>
        <Text type="secondary">这里是你的转发控制面概览</Text>
      </div>

      <Row gutter={[16, 16]} style={{ marginBottom: 24 }}>
        <Col xs={12} md={6}>
          <Link to="/forwards">
            <Card hoverable size="small">
              <Statistic
                title={<Space size={6}><SwapOutlined />我的转发</Space>}
                value={fws?.length ?? 0}
                suffix="条"
                loading={fws === null}
              />
            </Card>
          </Link>
        </Col>
        {user?.role === "admin" && (
          <>
            <Col xs={12} md={6}>
              <Link to="/nodes">
                <Card hoverable size="small">
                  <Statistic
                    title={<Space size={6}><DatabaseOutlined />在线节点</Space>}
                    value={sla?.online ?? 0}
                    suffix={`/ ${sla?.total ?? 0}`}
                    loading={!sla}
                    valueStyle={{ color: sla && sla.online === sla.total ? "#52c41a" : "#faad14" }}
                  />
                </Card>
              </Link>
            </Col>
            <Col xs={12} md={6}>
              <Link to="/sla">
                <Card hoverable size="small">
                  <Statistic
                    title={<Space size={6}><AreaChartOutlined />平均可用率</Space>}
                    value={avgUptime ?? 0}
                    precision={1}
                    suffix="%"
                    loading={!sla}
                    valueStyle={{ color: "#1677ff" }}
                  />
                </Card>
              </Link>
            </Col>
            <Col xs={12} md={6}>
              <Link to="/sla">
                <Card hoverable size="small">
                  <Statistic
                    title={<Space size={6}><AlertOutlined />故障事件</Space>}
                    value={totalFails}
                    loading={!sla}
                    valueStyle={{ color: totalFails > 0 ? "#ff4d4f" : undefined }}
                  />
                </Card>
              </Link>
            </Col>
          </>
        )}
      </Row>

      <Card
        title="最近转发"
        extra={
          <Link to="/forwards/new">
            <Button type="primary" icon={<PlusOutlined />}>新建转发</Button>
          </Link>
        }
      >
        {fws === null ? (
          <Skeleton active />
        ) : fws.length === 0 ? (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="还没有转发"
          >
            <Link to="/forwards/new">
              <Button type="primary" icon={<PlusOutlined />}>创建第一条转发</Button>
            </Link>
          </Empty>
        ) : (
          <List
            dataSource={fws.slice(0, 6)}
            renderItem={(f) => (
              <List.Item
                actions={[
                  <Link to={`/forwards/${f.id}/edit`} key="view">
                    查看 <ArrowRightOutlined />
                  </Link>,
                ]}
              >
                <List.Item.Meta
                  title={
                    <Space>
                      <Text strong>{f.name}</Text>
                      <Tag color="blue" className="num">:{f.listen_port}</Tag>
                    </Space>
                  }
                  description={<HopsLine f={f} />}
                />
              </List.Item>
            )}
          />
        )}
      </Card>
    </div>
  );
}

function HopsLine({ f }: { f: Forward }) {
  return (
    <Space size={6} className="num" style={{ fontSize: 12 }}>
      {f.hops.map((h, hi) => {
        const ids = h.nodes.map((n) => n.id).join(",");
        const txt = h.nodes.length === 1 ? h.nodes[0].id : `[${ids}]`;
        return (
          <span key={hi}>
            {hi > 0 && <span style={{ color: "#bfbfbf", margin: "0 4px" }}>→</span>}
            <span style={{ color: hi === 0 ? "#1677ff" : "#595959" }}>{txt}</span>
          </span>
        );
      })}
      <span style={{ color: "#bfbfbf", margin: "0 4px" }}>→</span>
      <Text type="secondary" style={{ fontSize: 12 }}>{f.target}</Text>
    </Space>
  );
}
