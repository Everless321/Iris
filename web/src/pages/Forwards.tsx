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
      width: 160,
      ellipsis: true,
      render: (name, f) => (
        <Link to={`/admin/forwards/${f.id}`}>
          <Text strong ellipsis={{ tooltip: name }} style={{ maxWidth: 140 }}>{name}</Text>
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
      width: 130,
      render: (p: string, f) => {
        const parts = p.split("+").map((x) => x.trim().toUpperCase()).filter(Boolean);
        return (
          <>
            {parts.map((x) => <Tag key={x}>{x}</Tag>)}
            {f.link_encryption === "plain" && (
              <Tooltip title="节点间走明文 TCP（同机房 / 信任内网）— 仅对 TCP 跳生效">
                <Tag color="warning" style={{ marginLeft: 4, cursor: "help" }}>明文</Tag>
              </Tooltip>
            )}
            {f.path_mode === "fast" && (
              <Tooltip title="管理员配置：强制内核 fast path（nftables DNAT）。失败自动回退 slow path — 看「Listener」列实际路径。">
                <Tag color="processing" style={{ marginLeft: 4, cursor: "help" }}>fast*</Tag>
              </Tooltip>
            )}
            {f.path_mode === "slow" && (
              <Tooltip title="管理员配置：强制 slow path（用户态 tokio）— 保留 session 历史 + 双向流量统计准确">
                <Tag style={{ marginLeft: 4, cursor: "help" }}>slow*</Tag>
              </Tooltip>
            )}
          </>
        );
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
      title: "配额",
      key: "quota",
      width: 150,
      render: (_, f) => <QuotaCell f={f} />,
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
          <Link to={`/admin/forwards/${f.id}/edit`}>
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
        <Link to="/admin/forwards/new">
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
                <Link to="/admin/forwards/new">
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
  const baseColor = allOk ? "success" : allFail ? "error" : "warning";
  const label = allOk ? `运行 ${okCount}/${total}` : `${okCount}/${total} 正常`;

  // M4.4 配置 vs 实际不匹配检测
  const intent = f.path_mode ?? "auto";
  const fastCount = states.filter((s) => s.actual_path === "fast").length;
  const slowCount = states.filter((s) => (s.actual_path ?? "") !== "fast").length;
  // path_mode=fast 但 有节点跑 slow（fallback 了）
  const fastFellBack = intent === "fast" && fastCount < total;
  // path_mode=slow 不会变 fast，不冲突
  // path_mode=auto + 异构 = 信息性，非异常

  const tip = (
    <div style={{ fontSize: 12, maxWidth: 360 }}>
      {fastFellBack && (
        <div style={{ marginBottom: 6, padding: 4, background: "#5b2c2c", borderRadius: 4 }}>
          <Tag color="warning" style={{ margin: 0, marginRight: 4, fontSize: 11 }}>fast 回退</Tag>
          配置 path_mode=fast，但 {total - fastCount}/{total} 节点实际 slow
        </div>
      )}
      {states.map((s) => {
        const ap = s.actual_path ?? "";
        const apLabel = ap === "fast" ? "fast" : ap === "slow" ? "slow" : "?";
        const apColor: "blue" | "default" | "warning" =
          ap === "fast" ? "blue" : ap === "slow" ? "default" : "warning";
        return (
          <div key={s.node_id} style={{ marginBottom: 4 }}>
            <Tag color={s.ok ? "success" : "error"} style={{ margin: 0, marginRight: 4, fontSize: 11 }}>
              {s.ok ? "OK" : "FAIL"}
            </Tag>
            <Tag color={apColor} style={{ margin: 0, marginRight: 6, fontSize: 11 }}>
              {apLabel}
            </Tag>
            <span style={{ fontFamily: "monospace" }}>{s.node_id}</span>
            {!s.ok && s.error && (
              <div style={{ color: "#ff7875", marginTop: 2, fontFamily: "monospace", fontSize: 11 }}>
                {s.error}
              </div>
            )}
          </div>
        );
      })}
      {!fastFellBack && fastCount > 0 && slowCount > 0 && (
        <div style={{ marginTop: 4, color: "#999", fontSize: 11 }}>
          多入口节点能力不同 → 混合路径运行（path_mode={intent}）
        </div>
      )}
    </div>
  );
  return (
    <Tooltip title={tip} placement="left">
      <Tag color={fastFellBack ? "warning" : baseColor}
           style={{ margin: 0, fontSize: 11, cursor: "help" }}>
        {fastFellBack ? `⚠ ${label}` : label}
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

/// #39 配额状态 cell：耗尽红 tag / 接近 80% 黄 / 正常绿 / 无限灰；含速率 / 重置倒计时 tooltip。
function QuotaCell({ f }: { f: Forward }) {
  const qin = f.quota_in_bytes ?? null;
  const qout = f.quota_out_bytes ?? null;
  const rin = f.rate_in_bps ?? null;
  const rout = f.rate_out_bps ?? null;
  const exhausted = f.quota_exhausted_at_ms != null;
  const hasAny = qin != null || qout != null || rin != null || rout != null;

  if (!hasAny) {
    return <Text type="secondary" style={{ fontSize: 12 }}>—</Text>;
  }

  const usedIn = f.bytes_in ?? 0;
  const usedOut = f.bytes_out ?? 0;
  const pctIn = qin ? Math.min(100, Math.round((usedIn / qin) * 100)) : 0;
  const pctOut = qout ? Math.min(100, Math.round((usedOut / qout) * 100)) : 0;
  const worst = Math.max(pctIn, pctOut);

  let color: "success" | "warning" | "error" | "default" = "success";
  let label = "正常";
  if (exhausted) { color = "error"; label = "已耗尽"; }
  else if (worst >= 80 && (qin || qout)) { color = "warning"; label = `${worst}% 已用`; }
  else if (qin || qout) { color = "success"; label = `${worst}% 已用`; }
  else { color = "default"; label = "仅限速"; }

  const tip = (
    <div style={{ fontSize: 12, lineHeight: 1.7 }}>
      {qin != null && (
        <div>上传 quota: {formatBytesGB(usedIn)} / {formatBytesGB(qin)}</div>
      )}
      {qout != null && (
        <div>下载 quota: {formatBytesGB(usedOut)} / {formatBytesGB(qout)}</div>
      )}
      {rin != null && rin > 0 && <div>上传带宽: {formatRate(rin)}</div>}
      {rout != null && rout > 0 && <div>下载带宽: {formatRate(rout)}</div>}
      {f.quota_reset && (
        <div>重置策略: {f.quota_reset === "daily" ? "每日 UTC 00:00" : "每月 1 号 UTC 00:00"}</div>
      )}
      {f.quota_reset_at_ms != null && (
        <div style={{ marginTop: 4, color: "#999" }}>
          下次重置: {new Date(f.quota_reset_at_ms).toLocaleString()}
        </div>
      )}
      {exhausted && f.quota_exhausted_at_ms != null && (
        <div style={{ marginTop: 4, color: "#ff7875" }}>
          软停于 {new Date(f.quota_exhausted_at_ms).toLocaleString()}
        </div>
      )}
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

function formatBytesGB(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 ** 2) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1024 ** 3) return `${(b / 1024 ** 2).toFixed(1)} MB`;
  return `${(b / 1024 ** 3).toFixed(2)} GB`;
}

function formatRate(bps: number): string {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 ** 2) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / 1024 ** 2).toFixed(2)} MB/s`;
}
