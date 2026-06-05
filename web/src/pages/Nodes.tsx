import { useEffect, useState } from "react";
import {
  Table, Button, Card, Tag, Space, Typography, Popconfirm, Modal, Form, Input,
  InputNumber, App, Alert, Tooltip, Skeleton, Empty,
} from "antd";
import {
  PlusOutlined, DeleteOutlined, ReloadOutlined, CopyOutlined,
  CheckCircleFilled, MinusCircleFilled, QuestionCircleFilled,
  DashboardOutlined, CloudUploadOutlined,
} from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import { api, type Enrollment, type Node } from "../lib/api";
import NodeMetricsModal from "./NodeMetricsModal";
import NodeUpgradeModal from "./NodeUpgradeModal";

const { Title, Text, Paragraph } = Typography;

function HealthTag({ h }: { h: string }) {
  if (h === "healthy") return <Tag icon={<CheckCircleFilled />} color="success">在线</Tag>;
  if (h === "unhealthy") return <Tag icon={<MinusCircleFilled />} color="error">离线</Tag>;
  return <Tag icon={<QuestionCircleFilled />} color="default">未知</Tag>;
}

// 节点 mTLS cert 剩余天数。剩 ≤30 天节点会自动调 RenewCert RPC；
// UI 仅展示 / 在续签失败时提示运维。
// M4.4 节点 fast path 能力徽章。capabilities 字段是 node heartbeat 上报的 JSON。
// 老节点 (空字符串) 视为不支持 → 灰色 — 表明 fast path 永远 fall back slow。
function FastPathBadge({ caps }: { caps?: string }) {
  if (!caps) {
    return (
      <Tooltip title="节点未上报能力（老节点 / 未升级 M4.2 binary）。该节点上所有 forward 将走 slow path。">
        <Tag color="default" style={{ margin: 0, cursor: "help", fontSize: 11 }}>—</Tag>
      </Tooltip>
    );
  }
  let info: { fastpath: boolean; reason: string; kernel: string; in_container: boolean };
  try {
    info = JSON.parse(caps);
  } catch {
    return <Tag color="default" style={{ margin: 0, fontSize: 11 }}>解析失败</Tag>;
  }
  const reasonMap: Record<string, string> = {
    "ok": "已就绪",
    "non-linux": "非 Linux 系统",
    "kernel-too-old": "内核版本过低 (需 ≥ 5.4)",
    "nft-binary-missing": "缺少 nft 二进制",
    "container-network-untrusted": "容器内 (Docker bridge / LXC)",
    "missing-CAP_NET_ADMIN": "无 CAP_NET_ADMIN 权限",
  };
  // 探测 reason 不在 map → 显示原文（dry-run-failed: ...）
  const reasonText = reasonMap[info.reason] ?? info.reason ?? "未知";
  const tip = (
    <div style={{ fontSize: 12, lineHeight: 1.6 }}>
      <div>状态：<strong>{info.fastpath ? "✓ 支持" : "✗ 不支持"}</strong></div>
      <div>原因：{reasonText}</div>
      {info.kernel && <div>内核：{info.kernel}</div>}
      {info.in_container && <div style={{ color: "#faad14" }}>⚠ 容器环境</div>}
      <div style={{ marginTop: 4, color: "#999", fontSize: 11 }}>
        {info.fastpath
          ? "该节点的单跳 + 非 TLS forward 可走内核 nftables DNAT（CPU ~0%）"
          : "该节点所有 forward 永远走用户态 tokio"}
      </div>
    </div>
  );
  return (
    <Tooltip title={tip} placement="left">
      <Tag color={info.fastpath ? "success" : "default"} style={{ margin: 0, cursor: "help", fontSize: 11 }}>
        {info.fastpath ? "✓ 支持" : "✗ 不支持"}
      </Tag>
    </Tooltip>
  );
}

function VersionBadge({ nodeVer, masterVer }: { nodeVer?: string; masterVer: string | null }) {
  if (!nodeVer) {
    return <Tag color="default" style={{ margin: 0 }}>—</Tag>;
  }
  const short = nodeVer.split("-").pop() || nodeVer;
  if (!masterVer) {
    return <Tag style={{ margin: 0, fontFamily: "monospace" }}>{short}</Tag>;
  }
  if (nodeVer === masterVer) {
    return (
      <Tooltip title={nodeVer}>
        <Tag color="success" style={{ margin: 0, fontFamily: "monospace" }}>{short} ✓</Tag>
      </Tooltip>
    );
  }
  return (
    <Tooltip title={`节点 ${nodeVer} · master ${masterVer}`}>
      <Tag color="warning" style={{ margin: 0, fontFamily: "monospace" }}>{short} ⚠</Tag>
    </Tooltip>
  );
}

function CertExpiryBadge({ notAfter }: { notAfter?: number }) {
  if (!notAfter || notAfter <= 0) {
    return <Tag color="default" style={{ margin: 0 }}>—</Tag>;
  }
  const remainingMs = notAfter - Date.now();
  const remainingDays = Math.floor(remainingMs / (24 * 3600 * 1000));
  if (remainingDays <= 7) {
    return <Tag color="error" style={{ margin: 0, fontFamily: "inherit" }}>{remainingDays}d ❌</Tag>;
  }
  if (remainingDays <= 30) {
    return <Tag color="warning" style={{ margin: 0, fontFamily: "inherit" }}>{remainingDays}d ⚠</Tag>;
  }
  return <Tag color="success" style={{ margin: 0, fontFamily: "inherit" }}>{remainingDays}d</Tag>;
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
  // 脚本本体直接从 GitHub raw 拉（HTTPS 自带签名 + 可缓存），
  // master 的 /install.sh 只是 307 跳到这里，不必再绕一圈。
  const cmd = `curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | bash -s -- \\
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
  const [metricsNodeId, setMetricsNodeId] = useState<string | null>(null);
  const [upgradeNodeId, setUpgradeNodeId] = useState<string | null>(null);
  const [masterVersion, setMasterVersion] = useState<string | null>(null);
  const { message } = App.useApp();

  const load = () => api.get<Node[]>("/api/nodes").then(setList).catch(() => setList([]));

  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    // M8 master version：拿来给 VersionBadge 比对。一次性，不变。
    api.get<{ version: string }>("/api/version")
      .then((v) => setMasterVersion(v.version))
      .catch(() => setMasterVersion(null));
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
    { title: "名称", dataIndex: "name", key: "name", width: 160, ellipsis: true, render: (n) => <Text strong>{n}</Text> },
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
      title: "证书到期", key: "cert_expiry", width: 110,
      render: (_, n) => <CertExpiryBadge notAfter={n.cert_not_after_ms} />,
    },
    {
      title: "版本", key: "version", width: 120,
      render: (_, n) => <VersionBadge nodeVer={n.version} masterVer={masterVersion} />,
    },
    {
      title: "操作", key: "actions", width: 200, align: "right",
      render: (_, n) => (
        <Space size={4}>
          <Tooltip title="查看 CPU/内存/磁盘/网速 实时监控">
            <Button type="link" size="small" icon={<DashboardOutlined />} onClick={() => setMetricsNodeId(n.id)}>
              监控
            </Button>
          </Tooltip>
          <Popconfirm
            title={`远程升级 ${n.id}?`}
            description="节点会下载最新 rolling binary 并自动 swap + restart。失败会自动回滚 .bak"
            okText="升级" cancelText="取消"
            onConfirm={() => setUpgradeNodeId(n.id)}
          >
            <Tooltip title="拉取最新 binary 并升级（自带 60s watchdog 自愈）">
              <Button type="link" size="small" icon={<CloudUploadOutlined />}>升级</Button>
            </Tooltip>
          </Popconfirm>
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
            scroll={{ x: "max-content" }}
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

      <NodeMetricsModal
        nodeId={metricsNodeId}
        onClose={() => setMetricsNodeId(null)}
      />

      <NodeUpgradeModal
        nodeId={upgradeNodeId}
        open={upgradeNodeId !== null}
        onClose={() => setUpgradeNodeId(null)}
      />
    </div>
  );
}
