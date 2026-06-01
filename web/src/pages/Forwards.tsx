import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  Table, Button, Card, Tag, Space, Typography, Popconfirm, Empty, App, Tooltip,
} from "antd";
import { PlusOutlined, EditOutlined, DeleteOutlined } from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import { api, type Forward } from "../lib/api";

const { Title, Text } = Typography;

export default function Forwards() {
  const [list, setList] = useState<Forward[] | null>(null);
  const { message } = App.useApp();

  const load = () => api.get<Forward[]>("/api/forwards").then(setList).catch(() => setList([]));
  useEffect(() => { load(); }, []);

  async function onDel(id: number) {
    try {
      await api.del(`/api/forwards/${id}`);
      message.success("已删除");
      load();
    } catch (e: any) {
      message.error(e.message);
    }
  }

  const columns: ColumnsType<Forward> = [
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      render: (name, f) => (
        <Link to={`/forwards/${f.id}/edit`}>
          <Text strong>{name}</Text>
        </Link>
      ),
    },
    {
      title: "监听端口",
      dataIndex: "listen_port",
      key: "listen_port",
      width: 110,
      render: (p) => <Tag color="blue" className="num">:{p}</Tag>,
    },
    {
      title: "协议",
      dataIndex: "protocol",
      key: "protocol",
      width: 110,
      render: (p: string) => {
        const parts = p.split("+").map((x) => x.trim().toUpperCase()).filter(Boolean);
        return <>{parts.map((x) => <Tag key={x}>{x}</Tag>)}</>;
      },
    },
    {
      title: "路径",
      key: "hops",
      render: (_, f) => <PathInline f={f} />,
    },
    {
      title: "Listener",
      key: "listener_status",
      width: 130,
      render: (_, f) => <ListenerBadge f={f} />,
    },
    {
      title: "流量",
      key: "traffic",
      width: 170,
      render: (_, f) => <TrafficCell f={f} />,
    },
    {
      title: "目标",
      key: "targets",
      width: 220,
      render: (_, f) => {
        const ts = f.targets ?? [];
        if (ts.length === 0) return <Text type="secondary">—</Text>;
        const head = ts[0]?.addr ?? "";
        return (
          <Text className="num" type="secondary" style={{ fontSize: 12 }}>
            {head}{ts.length > 1 ? ` +${ts.length - 1}` : ""}
          </Text>
        );
      },
    },
    {
      title: "操作",
      key: "actions",
      width: 140,
      align: "right",
      render: (_, f) => (
        <Space size={4}>
          <Link to={`/forwards/${f.id}/edit`}>
            <Button type="link" size="small" icon={<EditOutlined />}>编辑</Button>
          </Link>
          <Popconfirm
            title={`删除转发 #${f.id}?`}
            okText="删除"
            okType="danger"
            cancelText="取消"
            onConfirm={() => onDel(f.id)}
          >
            <Button type="link" size="small" danger icon={<DeleteOutlined />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div style={{ maxWidth: 1280, margin: "0 auto" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 24 }}>
        <div>
          <Title level={3} style={{ marginBottom: 4 }}>转发管理</Title>
          <Text type="secondary">查看、创建、编辑你的转发规则</Text>
        </div>
        <Link to="/forwards/new">
          <Button type="primary" icon={<PlusOutlined />} size="large">新建转发</Button>
        </Link>
      </div>

      <Card>
        <Table<Forward>
          rowKey="id"
          loading={list === null}
          dataSource={list ?? []}
          columns={columns}
          pagination={{ pageSize: 10, showSizeChanger: false, hideOnSinglePage: true }}
          locale={{
            emptyText: (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description="还没有转发"
              >
                <Link to="/forwards/new">
                  <Button type="primary" icon={<PlusOutlined />}>创建第一条</Button>
                </Link>
              </Empty>
            ),
          }}
        />
      </Card>
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let v = n / 1024;
  for (const u of units) {
    if (v < 1024) return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${u}`;
    v /= 1024;
  }
  return `${v.toFixed(2)} EB`;
}

function TrafficCell({ f }: { f: Forward }) {
  const bin = f.bytes_in ?? 0;
  const bout = f.bytes_out ?? 0;
  if (bin === 0 && bout === 0) {
    return <Text type="secondary" style={{ fontSize: 12 }}>—</Text>;
  }
  return (
    <Tooltip
      title={
        <div style={{ fontSize: 12, fontFamily: "monospace" }}>
          <div>↑ 上行: {bin.toLocaleString()} B</div>
          <div>↓ 下行: {bout.toLocaleString()} B</div>
          <div style={{ marginTop: 4, color: "#999" }}>合计: {(bin + bout).toLocaleString()} B</div>
        </div>
      }
    >
      <Space size={4} direction="vertical" style={{ fontSize: 11, cursor: "help" }}>
        <Text className="num" style={{ fontSize: 11 }}>↑ {formatBytes(bin)}</Text>
        <Text className="num" type="secondary" style={{ fontSize: 11 }}>↓ {formatBytes(bout)}</Text>
      </Space>
    </Tooltip>
  );
}

function ListenerBadge({ f }: { f: Forward }) {
  const states = f.listener_status ?? [];
  if (states.length === 0) {
    return <Tag color="default" style={{ margin: 0, fontSize: 11 }}>等待心跳</Tag>;
  }
  const okCount = states.filter((s) => s.ok).length;
  const total = states.length;
  const allOk = okCount === total;
  const allFail = okCount === 0;
  const color = allOk ? "success" : allFail ? "error" : "warning";
  const label = allOk ? `运行 ${okCount}/${total}` : `${okCount}/${total} 正常`;
  const tip = (
    <div style={{ fontSize: 12, maxWidth: 320 }}>
      {states.map((s) => (
        <div key={s.node_id} style={{ marginBottom: 4 }}>
          <Tag color={s.ok ? "success" : "error"} style={{ margin: 0, marginRight: 6, fontSize: 11 }}>
            {s.ok ? "OK" : "FAIL"}
          </Tag>
          <span style={{ fontFamily: "monospace" }}>{s.node_id}</span>
          {!s.ok && s.error && (
            <div style={{ color: "#ff7875", marginTop: 2, fontFamily: "monospace", fontSize: 11 }}>
              {s.error}
            </div>
          )}
        </div>
      ))}
    </div>
  );
  return (
    <Tooltip title={tip} placement="left">
      <Tag color={color} style={{ margin: 0, fontSize: 11, cursor: "help" }}>
        {label}
      </Tag>
    </Tooltip>
  );
}

function PathInline({ f }: { f: Forward }) {
  return (
    <Space size={4} className="num" style={{ fontSize: 12, flexWrap: "wrap" }}>
      {f.hops.map((h, hi) => {
        const ids = h.nodes.map((n) => (n.weight > 1 ? `${n.id}:${n.weight}` : n.id)).join(",");
        const txt = h.nodes.length === 1 ? h.nodes[0].id : `[${ids}]`;
        return (
          <span key={hi} style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
            {hi > 0 && <span style={{ color: "#bfbfbf" }}>→</span>}
            <Tag
              color={hi === 0 ? "blue" : "default"}
              style={{ margin: 0, fontFamily: "inherit" }}
            >
              {txt}
            </Tag>
            {hi > 0 && h.nodes.length > 1 && (
              <span style={{ color: "#bfbfbf", fontSize: 10 }}>@{h.strategy}</span>
            )}
          </span>
        );
      })}
    </Space>
  );
}
