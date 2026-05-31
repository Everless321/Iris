import { useEffect, useState } from "react";
import {
  Table, Button, Card, Tag, Space, Typography, Popconfirm, Modal, Form, Input,
  InputNumber, App, Alert, Tooltip, Skeleton, Empty,
} from "antd";
import {
  PlusOutlined, DeleteOutlined, ReloadOutlined, CopyOutlined,
  CheckCircleFilled, MinusCircleFilled, QuestionCircleFilled,
} from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import { api, type Enrollment, type Node } from "../lib/api";

const { Title, Text, Paragraph } = Typography;

function HealthTag({ h }: { h: string }) {
  if (h === "healthy") return <Tag icon={<CheckCircleFilled />} color="success">在线</Tag>;
  if (h === "unhealthy") return <Tag icon={<MinusCircleFilled />} color="error">离线</Tag>;
  return <Tag icon={<QuestionCircleFilled />} color="default">未知</Tag>;
}

function InstallDialog({
  open, enrollment, onClose,
}: {
  open: boolean; enrollment: Enrollment | null; onClose: () => void;
}) {
  const { message } = App.useApp();
  if (!enrollment) return null;
  const masterUrl = `${location.protocol}//${location.host}`;
  const isLocal = /^(localhost|127\.0\.0\.1|\[::1\])(:|$)/.test(location.host);
  const isInsecure = location.protocol === "http:" && !isLocal;
  const cmd = `curl -fsSL ${masterUrl}/install.sh | bash -s -- \\
  --master ${masterUrl} \\
  --token ${enrollment.token}`;

  function copy() {
    navigator.clipboard.writeText(cmd);
    message.success("命令已复制");
  }

  return (
    <Modal
      open={open}
      onCancel={onClose}
      footer={
        <Button type="primary" onClick={copy} icon={<CopyOutlined />}>
          复制命令
        </Button>
      }
      title={
        <Space>
          <span>节点安装命令</span>
          <Tag color="blue" className="num">{enrollment.node_id}</Tag>
        </Space>
      }
      width={620}
    >
      <Paragraph type="secondary" style={{ marginBottom: 16 }}>
        SSH 到目标服务器后粘贴执行。脚本会自动兑换证书、写入配置、启动节点。
      </Paragraph>

      {isInsecure && (
        <Alert
          type="warning"
          showIcon
          message="当前链路不安全"
          description="你正通过 HTTP 访问 master。生产部署请套上 HTTPS 并在 master 设置 IRIS_REQUIRE_TLS=1。"
          style={{ marginBottom: 16 }}
        />
      )}

      <Input.TextArea
        value={cmd}
        autoSize={{ minRows: 3 }}
        readOnly
        style={{ fontFamily: "JetBrains Mono, monospace", fontSize: 12, marginBottom: 16 }}
      />

      <div style={{ background: "#fafafa", border: "1px solid #f0f0f0", borderRadius: 8, padding: 12 }}>
        <Space direction="vertical" size={6} style={{ width: "100%" }}>
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
            <Text type="secondary">令牌</Text>
            <Text className="num" copyable={{ text: enrollment.token }}>
              {enrollment.token.slice(0, 12)}…
            </Text>
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12 }}>
            <Text type="secondary">有效期至</Text>
            <Text className="num">{new Date(enrollment.expires_at).toLocaleString()}</Text>
          </div>
        </Space>
      </div>

      <Alert
        type="info"
        showIcon
        style={{ marginTop: 12 }}
        message="令牌一次性 + 24h 失效，过期请在节点行点【重发令牌】。"
      />
    </Modal>
  );
}

export default function Nodes() {
  const [list, setList] = useState<Node[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null);
  const [busy, setBusy] = useState(false);
  const { message } = App.useApp();

  const load = () => api.get<Node[]>("/api/nodes").then(setList).catch(() => setList([]));

  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  async function onAdd(v: { id: string; name: string; addr: string; weight: number }) {
    setBusy(true);
    try {
      await api.post("/api/nodes", v);
      const tok = await api.post<Enrollment>(`/api/nodes/${v.id}/enrollment`);
      setEnrollment(tok);
      setAdding(false);
      form.resetFields();
      load();
      message.success("节点已创建");
    } catch (e: any) {
      message.error(e.message);
    } finally {
      setBusy(false);
    }
  }

  async function regenToken(id: string) {
    try {
      const tok = await api.post<Enrollment>(`/api/nodes/${id}/enrollment`);
      setEnrollment(tok);
    } catch (e: any) {
      message.error(e.message);
    }
  }

  async function onDel(id: string) {
    try {
      await api.del(`/api/nodes/${id}`);
      message.success("已删除");
      load();
    } catch (e: any) {
      message.error(e.message);
    }
  }

  const columns: ColumnsType<Node> = [
    { title: "ID", dataIndex: "id", key: "id", width: 120, render: (id) => <Text className="num">{id}</Text> },
    { title: "名称", dataIndex: "name", key: "name", render: (n) => <Text strong>{n}</Text> },
    { title: "公网地址", dataIndex: "addr", key: "addr", width: 200, render: (a) => <Text className="num" type="secondary">{a}</Text> },
    { title: "健康", dataIndex: "health", key: "health", width: 100, render: (h) => <HealthTag h={h} /> },
    {
      title: "延迟", dataIndex: "latency_ms", key: "latency", width: 100,
      render: (l) => l != null ? <Text className="num">{l}ms</Text> : <Text type="secondary">—</Text>,
    },
    { title: "权重", dataIndex: "weight", key: "weight", width: 80, render: (w) => <Text className="num">{w}</Text> },
    {
      title: "可用率", key: "uptime", width: 100,
      render: (_, n) => n.probe_total > 0
        ? <Text className="num">{((n.probe_ok / n.probe_total) * 100).toFixed(1)}%</Text>
        : <Text type="secondary">—</Text>,
    },
    {
      title: "操作", key: "actions", width: 200, align: "right",
      render: (_, n) => (
        <Space size={4}>
          <Tooltip title="重新生成安装令牌">
            <Button type="link" size="small" icon={<ReloadOutlined />} onClick={() => regenToken(n.id)}>
              重发令牌
            </Button>
          </Tooltip>
          <Popconfirm
            title={`删除节点 ${n.id}?`}
            okText="删除" okType="danger" cancelText="取消"
            onConfirm={() => onDel(n.id)}
          >
            <Button type="link" size="small" danger icon={<DeleteOutlined />}>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div style={{ maxWidth: 1280, margin: "0 auto" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <div>
          <Title level={3} style={{ marginBottom: 4 }}>节点管理</Title>
          <Text type="secondary">添加节点后系统会生成一键安装命令，SSH 到目标服务器粘贴即可</Text>
        </div>
        <Button type="primary" size="large" icon={<PlusOutlined />} onClick={() => setAdding(true)}>
          新增节点
        </Button>
      </div>

      <Card>
        {list === null ? (
          <Skeleton active />
        ) : (
          <Table<Node>
            rowKey="id"
            dataSource={list}
            columns={columns}
            pagination={{ pageSize: 10, showSizeChanger: false, hideOnSinglePage: true }}
            locale={{
              emptyText: (
                <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有节点">
                  <Button type="primary" icon={<PlusOutlined />} onClick={() => setAdding(true)}>
                    新增第一个节点
                  </Button>
                </Empty>
              ),
            }}
          />
        )}
      </Card>

      {/* 新增节点 Modal */}
      <Modal
        open={adding}
        onCancel={() => setAdding(false)}
        title="新增节点"
        okText="创建并生成安装命令"
        cancelText="取消"
        confirmLoading={busy}
        onOk={() => form.submit()}
        width={520}
      >
        <Paragraph type="secondary" style={{ marginBottom: 16 }}>
          创建一个新的节点配置，系统会自动生成一次性安装命令
        </Paragraph>
        <Form
          form={form}
          layout="vertical"
          onFinish={onAdd}
          initialValues={{ weight: 1 }}
          requiredMark={false}
        >
          <Form.Item
            name="id"
            label="节点 ID"
            extra="全平台唯一短标识，如 sg-1、jp-tk-2"
            rules={[{ required: true, message: "请输入节点 ID" }]}
          >
            <Input placeholder="sg-1" className="num" />
          </Form.Item>
          <Form.Item
            name="name"
            label="节点名称"
            extra="显示名称，给自己看"
            rules={[{ required: true, message: "请输入节点名称" }]}
          >
            <Input placeholder="新加坡入口" />
          </Form.Item>
          <Form.Item
            name="addr"
            label="节点公网地址 (host:port)"
            extra="其它节点连接它使用的地址。家宽节点填映射后的外网 IP"
            rules={[{ required: true, message: "请输入节点地址" }]}
          >
            <Input placeholder="1.2.3.4:7444" className="num" />
          </Form.Item>
          <Form.Item
            name="weight"
            label="权重"
            extra="带宽大的填高，分流多"
          >
            <InputNumber min={1} max={1000} style={{ width: "100%" }} />
          </Form.Item>
        </Form>
      </Modal>

      <InstallDialog
        open={!!enrollment}
        enrollment={enrollment}
        onClose={() => setEnrollment(null)}
      />
    </div>
  );
}
