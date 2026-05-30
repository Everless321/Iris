import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
  Card, Form, Input, InputNumber, Select, Button, Space, Typography, Alert,
  Tag, Divider, App, Tooltip, Empty,
} from "antd";
import {
  ArrowLeftOutlined, SaveOutlined, PlusOutlined, DeleteOutlined,
  CloseOutlined, NodeIndexOutlined,
} from "@ant-design/icons";
import {
  ReactFlow, Background, Controls,
  type Node as RFNode, type Edge as RFEdge,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { api, type Forward, type Hop, type Node } from "../lib/api";
import { useAuth } from "../lib/auth";

const { Title, Text, Paragraph } = Typography;

const PATH_STRATEGIES = [
  { value: "weighted", label: "加权轮询" },
  { value: "source_hash", label: "会话保持（源IP哈希）" },
  { value: "least_conn", label: "最小连接数" },
  { value: "latency", label: "延迟最优" },
];

export default function TopologyEditor() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { user } = useAuth();
  const { message } = App.useApp();
  const readOnly = !!id && user?.role !== "admin";
  const [nodes, setNodes] = useState<Node[]>([]);
  const [form] = Form.useForm();
  // 表单基础字段单独管，hops 单独 state（动态复杂）
  const [hops, setHops] = useState<Hop[]>([{ strategy: "weighted", nodes: [] }]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.get<Node[]>("/api/nodes").catch(() => []).then((ns) => setNodes(ns as Node[]));
    if (id) {
      api.get<Forward[]>("/api/forwards").then((all) => {
        const f = all.find((x) => String(x.id) === id);
        if (f) {
          form.setFieldsValue({
            name: f.name,
            listen_port: f.listen_port,
            protocol: f.protocol,
            target: f.target,
          });
          setHops(f.hops);
        }
      });
    } else {
      form.setFieldsValue({ protocol: "tcp", listen_port: 10080 });
    }
  }, [id, form]);

  // Entry ops (hops[0])
  const addEntryNode = (nid: string) => {
    if (!nid) return;
    setHops((hs) => {
      const entry = hs[0] ?? { strategy: "weighted", nodes: [] };
      if (entry.nodes.some((n) => n.id === nid)) return hs;
      return [{ ...entry, nodes: [...entry.nodes, { id: nid, weight: 1 }] }, ...hs.slice(1)];
    });
  };
  const rmEntryNode = (i: number) =>
    setHops((hs) => [{ ...hs[0], nodes: hs[0].nodes.filter((_, x) => x !== i) }, ...hs.slice(1)]);

  // Path ops (hops[1..])
  const addPathHop = () => setHops((hs) => [...hs, { strategy: "weighted", nodes: [] }]);
  const rmPathHop = (pi: number) => setHops((hs) => hs.filter((_, x) => x !== pi + 1));
  const setPathStrategy = (pi: number, s: string) =>
    setHops((hs) => hs.map((h, x) => (x === pi + 1 ? { ...h, strategy: s } : h)));
  const addPathNode = (pi: number, nid: string) => {
    if (!nid) return;
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1 && !h.nodes.some((n) => n.id === nid)
          ? { ...h, nodes: [...h.nodes, { id: nid, weight: 1 }] }
          : h
      )
    );
  };
  const setPathWeight = (pi: number, ni: number, w: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1
          ? { ...h, nodes: h.nodes.map((n, y) => (y === ni ? { ...n, weight: w } : n)) }
          : h
      )
    );
  const rmPathNode = (pi: number, ni: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1 ? { ...h, nodes: h.nodes.filter((_, y) => y !== ni) } : h
      )
    );

  // 表单基础字段
  const target = Form.useWatch("target", form);
  const listen = Form.useWatch("listen_port", form);

  // ReactFlow 视图（light 主题）
  const { rfNodes, rfEdges } = useMemo(() => {
    const rn: RFNode[] = [];
    const re: RFEdge[] = [];
    hops.forEach((h, hi) => {
      const isEntry = hi === 0;
      const isExit = hi === hops.length - 1;
      const label = isEntry ? "入口（空）" : isExit ? "出口（空）" : `第 ${hi} 跳（空）`;
      if (h.nodes.length === 0) {
        rn.push({
          id: `h${hi}-empty`,
          data: { label },
          position: { x: hi * 220, y: 80 },
          style: {
            background: "#fafafa", border: "1px dashed #d9d9d9",
            color: "#bfbfbf", borderRadius: 8, fontSize: 11, padding: 8,
          },
        });
      }
      h.nodes.forEach((n, ni) => {
        rn.push({
          id: `h${hi}-${n.id}`,
          data: {
            label: (
              <div style={{ fontSize: 12 }}>
                <div style={{ fontWeight: 600, fontFamily: "JetBrains Mono, monospace" }}>{n.id}</div>
                <div style={{ color: "#8c8c8c", fontSize: 10, marginTop: 2 }}>
                  {isEntry ? "入口" : `w=${n.weight}`}
                </div>
              </div>
            ) as any,
          },
          position: { x: hi * 220, y: ni * 70 + 20 },
          style: {
            background: "#fff",
            border: `2px solid ${isEntry ? "#1677ff" : "#d9d9d9"}`,
            color: "#1f1f1f",
            borderRadius: 8,
            padding: 8,
            width: 130,
            boxShadow: "0 2px 6px rgba(0,0,0,0.04)",
          },
        });
      });
    });
    for (let i = 0; i < hops.length - 1; i++) {
      const left = hops[i].nodes.length ? hops[i].nodes.map((n) => `h${i}-${n.id}`) : [`h${i}-empty`];
      const right = hops[i + 1].nodes.length
        ? hops[i + 1].nodes.map((n) => `h${i + 1}-${n.id}`)
        : [`h${i + 1}-empty`];
      for (const a of left)
        for (const b of right)
          re.push({ id: `${a}-${b}`, source: a, target: b, style: { stroke: "#d9d9d9" } });
    }
    if (target && hops.length > 0) {
      const last = hops[hops.length - 1];
      rn.push({
        id: "target",
        data: { label: `→ ${target}` },
        position: { x: hops.length * 220, y: 80 },
        style: {
          background: "#e6f4ff", border: "2px solid #1677ff",
          color: "#003eb3", borderRadius: 8, fontSize: 11, padding: 8,
          fontFamily: "JetBrains Mono, monospace",
        },
      });
      const sources = last.nodes.length
        ? last.nodes.map((n) => `h${hops.length - 1}-${n.id}`)
        : [`h${hops.length - 1}-empty`];
      for (const s of sources)
        re.push({ id: `${s}-target`, source: s, target: "target", style: { stroke: "#1677ff" } });
    }
    return { rfNodes: rn, rfEdges: re };
  }, [hops, target]);

  async function save() {
    try {
      const values = await form.validateFields();
      if (hops.some((h) => h.nodes.length === 0)) {
        message.error("每跳至少需要一个节点");
        return;
      }
      setBusy(true);
      const payload = { ...values, hops };
      if (id) await api.put(`/api/forwards/${id}`, payload);
      else await api.post("/api/forwards", payload);
      message.success(id ? "已保存修改" : "已创建");
      navigate("/forwards");
    } catch (e: any) {
      if (e?.errorFields) return; // 表单校验错误
      message.error(e.message);
    } finally {
      setBusy(false);
    }
  }

  const entry = hops[0] ?? { strategy: "weighted", nodes: [] };
  const pathHops = hops.slice(1);

  return (
    <div style={{ maxWidth: 1100, margin: "0 auto" }}>
      <Button
        type="link"
        icon={<ArrowLeftOutlined />}
        onClick={() => navigate("/forwards")}
        style={{ paddingLeft: 0, marginBottom: 8 }}
      >
        返回转发列表
      </Button>

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <div>
          <Title level={3} style={{ marginBottom: 4 }}>
            {id ? `编辑转发 #${id}` : "新建转发"}
          </Title>
          <Text type="secondary">
            选择入口节点和路径，平台会自动管理 mTLS 和故障转移
          </Text>
        </div>
        {!readOnly && (
          <Button type="primary" size="large" icon={<SaveOutlined />} loading={busy} onClick={save}>
            {id ? "保存修改" : "创建转发"}
          </Button>
        )}
      </div>

      <Form form={form} layout="vertical" disabled={readOnly} requiredMark={false}>
        {/* 基础信息 */}
        <Card title="基础配置" style={{ marginBottom: 16 }}>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 16 }}>
            <Form.Item
              name="name"
              label="转发名称"
              rules={[{ required: true, message: "请输入名称" }]}
            >
              <Input placeholder="比如：上海到日本游戏服" />
            </Form.Item>
            <Form.Item
              name="listen_port"
              label="监听端口"
              extra="所有入口节点都监听此端口"
              rules={[{ required: true, message: "请输入端口" }]}
            >
              <InputNumber min={1} max={65535} style={{ width: "100%" }} className="num" />
            </Form.Item>
            <Form.Item
              name="protocol"
              label="协议类型"
              rules={[{ required: true }]}
            >
              <Select
                options={[
                  { value: "tcp", label: "TCP" },
                  { value: "udp", label: "UDP" },
                ]}
              />
            </Form.Item>
            <Form.Item
              name="target"
              label="目标地址 (host:port)"
              extra="流量最终发到的目标地址"
              rules={[{ required: true, message: "请输入目标地址" }]}
            >
              <Input placeholder="1.2.3.4:22" className="num" />
            </Form.Item>
          </div>
        </Card>

        {/* 入口配置 */}
        <Card
          title={
            <Space>
              <span>入口节点</span>
              <Tag color="blue">客户端连接的地址</Tag>
            </Space>
          }
          extra={<Tag>{entry.nodes.length} 个入口</Tag>}
          style={{ marginBottom: 16 }}
        >
          <Alert
            type="info"
            showIcon
            message={
              <span>
                所有入口节点都监听 <Text strong className="num">:{listen || "—"}</Text>。
                客户端可连接其中<Text strong>任意一个</Text>的 IP（建议配 DNS A 记录多 IP）。
                入口的"挑选"在客户端那一端发生，平台不参与，因此<Text strong>入口段不需要 LB 策略</Text>。
              </span>
            }
            style={{ marginBottom: 16 }}
          />
          <Space wrap size={[8, 8]} style={{ width: "100%" }}>
            {entry.nodes.map((n, ni) => (
              <Tag
                key={n.id}
                color="blue"
                closable={!readOnly}
                onClose={() => rmEntryNode(ni)}
                style={{ padding: "4px 10px", fontSize: 13, fontFamily: "JetBrains Mono, monospace" }}
              >
                {n.id}
              </Tag>
            ))}
            {!readOnly && (
              <Select
                placeholder="+ 添加入口节点"
                value={undefined}
                style={{ minWidth: 180 }}
                onChange={addEntryNode}
                options={nodes
                  .filter((n) => !entry.nodes.some((x) => x.id === n.id))
                  .map((n) => ({ value: n.id, label: `${n.id} · ${n.name}` }))}
              />
            )}
          </Space>
        </Card>

        {/* 路径配置 */}
        <Card
          title={
            <Space>
              <span>路径节点</span>
              <Tag color="purple">中转 / 出口</Tag>
            </Space>
          }
          extra={
            !readOnly && (
              <Button type="primary" ghost icon={<PlusOutlined />} onClick={addPathHop}>
                添加下一跳
              </Button>
            )
          }
          style={{ marginBottom: 16 }}
        >
          {pathHops.length === 0 ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={
                <span style={{ fontSize: 13 }}>
                  单跳模式：入口节点直接发到 <Text className="num">{target || "目标地址"}</Text>
                </span>
              }
            >
              {!readOnly && (
                <Button type="primary" ghost icon={<PlusOutlined />} onClick={addPathHop}>
                  添加一跳
                </Button>
              )}
            </Empty>
          ) : (
            <Space direction="vertical" size={16} style={{ width: "100%" }}>
              {pathHops.map((h, pi) => {
                const isLast = pi === pathHops.length - 1;
                return (
                  <Card
                    key={pi}
                    type="inner"
                    size="small"
                    title={
                      <Space>
                        <NodeIndexOutlined />
                        <Text strong>{isLast ? `出口（最后一跳）` : `第 ${pi + 1} 跳（中转）`}</Text>
                      </Space>
                    }
                    extra={
                      <Space>
                        <Select
                          size="small"
                          value={h.strategy}
                          onChange={(v) => setPathStrategy(pi, v)}
                          options={PATH_STRATEGIES}
                          style={{ width: 180 }}
                          disabled={readOnly}
                        />
                        {!readOnly && (
                          <Tooltip title="删除此跳">
                            <Button
                              danger size="small"
                              icon={<DeleteOutlined />}
                              onClick={() => rmPathHop(pi)}
                            />
                          </Tooltip>
                        )}
                      </Space>
                    }
                  >
                    <Space wrap size={[8, 8]}>
                      {h.nodes.map((n, ni) => (
                        <Tag
                          key={n.id}
                          style={{
                            padding: "6px 10px", display: "inline-flex",
                            alignItems: "center", gap: 6, fontSize: 13,
                          }}
                        >
                          <span className="num">{n.id}</span>
                          <Divider type="vertical" style={{ margin: 0 }} />
                          <span style={{ fontSize: 11, color: "#8c8c8c" }}>w</span>
                          <InputNumber
                            size="small" min={1} max={1000}
                            value={n.weight}
                            disabled={readOnly}
                            onChange={(v) => setPathWeight(pi, ni, Number(v) || 1)}
                            style={{ width: 56 }}
                            className="num"
                          />
                          {!readOnly && (
                            <Button
                              type="text" size="small"
                              icon={<CloseOutlined />}
                              onClick={() => rmPathNode(pi, ni)}
                              style={{ marginLeft: 2 }}
                            />
                          )}
                        </Tag>
                      ))}
                      {!readOnly && (
                        <Select
                          placeholder="+ 添加节点"
                          value={undefined}
                          style={{ minWidth: 180 }}
                          onChange={(v) => v && addPathNode(pi, v as string)}
                          options={nodes
                            .filter((n) => !h.nodes.some((x) => x.id === n.id))
                            .map((n) => ({ value: n.id, label: `${n.id} · ${n.name}` }))}
                        />
                      )}
                    </Space>
                  </Card>
                );
              })}
            </Space>
          )}
        </Card>

        {/* 拓扑预览 */}
        <Card title="拓扑预览" style={{ marginBottom: 32 }}>
          <div style={{ height: 320, background: "#fafafa", borderRadius: 8 }}>
            <ReactFlow
              nodes={rfNodes} edges={rfEdges} fitView
              nodesDraggable={false} nodesConnectable={false} elementsSelectable={false}
              proOptions={{ hideAttribution: true }}
            >
              <Background color="#e8e8e8" gap={20} />
              <Controls showInteractive={false} />
            </ReactFlow>
          </div>
        </Card>
      </Form>
    </div>
  );
}
