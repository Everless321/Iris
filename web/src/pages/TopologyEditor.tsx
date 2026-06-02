import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
  Card, Form, Input, InputNumber, Select, Button, Space, Typography,
  App, Tag, Drawer, Popconfirm, Empty, Checkbox, Alert, Progress,
} from "antd";
import {
  ArrowLeftOutlined, SaveOutlined, PlusOutlined, CloseOutlined,
  DeleteOutlined, ThunderboltFilled, SwapOutlined, AimOutlined,
  ThunderboltOutlined, CheckCircleFilled, CloseCircleFilled,
} from "@ant-design/icons";
import {
  ReactFlow, Background, Controls, Handle, Position,
  BaseEdge, EdgeLabelRenderer, getBezierPath,
  useNodesState, useEdgesState,
  type Node as RFNode, type Edge as RFEdge,
  type NodeProps, type EdgeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  api, type EdgeProbe, type Forward, type Hop,
  type Node as ZNode, type TargetEndpoint, type TestResponse,
} from "../lib/api";
import { useAuth } from "../lib/auth";

const { Title, Text } = Typography;

const PATH_STRATEGIES = [
  { value: "weighted", label: "加权轮询" },
  { value: "source_hash", label: "会话保持" },
  { value: "least_conn", label: "最小连接" },
  { value: "latency", label: "延迟最优" },
];
const strategyLabel = (v: string) =>
  PATH_STRATEGIES.find((s) => s.value === v)?.label ?? v;

// ── 自定义节点 ─────────────────────────────────────────
type HopNodePayload = {
  hi: number;
  hop: Hop;
  listenPort?: number;
  readOnly: boolean;
  onPlusClick: (hi: number) => void;
  onDeleteClick: (hi: number) => void;
};
type TargetNodePayload = {
  targets: TargetEndpoint[];
  strategy: string;
  readOnly: boolean;
};

const handleStyle: React.CSSProperties = {
  width: 8, height: 8,
  background: "#fff",
  border: "1.5px solid #1677ff",
};

function EntryNode({ data, selected }: NodeProps) {
  const d = data as HopNodePayload;
  return (
    <NodeFrame
      accent="#1677ff"
      selected={!!selected}
      icon={<ThunderboltFilled style={{ color: "#1677ff", fontSize: 16 }} />}
      title="客户端入口"
      subtitle={`监听 :${d.listenPort ?? "—"}`}
      onPlusClick={!d.readOnly ? () => d.onPlusClick(d.hi) : undefined}
    >
      <NodeChips nodes={d.hop.nodes} accent="#1677ff" placeholder="点击此卡片选入口节点" />
      <Handle type="source" position={Position.Right} style={handleStyle} />
    </NodeFrame>
  );
}

function HopNode({ data, selected }: NodeProps) {
  const d = data as HopNodePayload;
  return (
    <NodeFrame
      accent="#722ed1"
      selected={!!selected}
      icon={<SwapOutlined style={{ color: "#722ed1", fontSize: 16 }} />}
      title={`中转 ${d.hi}`}
      subtitle={strategyLabel(d.hop.strategy)}
      onPlusClick={!d.readOnly ? () => d.onPlusClick(d.hi) : undefined}
      onDeleteClick={!d.readOnly ? () => d.onDeleteClick(d.hi) : undefined}
    >
      <NodeChips nodes={d.hop.nodes} accent="#722ed1" withWeight placeholder="点击此卡片选节点" />
      <Handle type="target" position={Position.Left} style={handleStyle} />
      <Handle type="source" position={Position.Right} style={handleStyle} />
    </NodeFrame>
  );
}

function TargetNode({ data, selected }: NodeProps) {
  const d = data as TargetNodePayload;
  const targets = d.targets ?? [];
  const first = targets[0]?.addr;
  return (
    <div
      style={{
        background: "#1677ff",
        color: "#fff",
        borderRadius: 12,
        padding: "14px 16px",
        width: 220,
        textAlign: "center",
        position: "relative",
        cursor: d.readOnly ? "default" : "pointer",
        boxShadow: selected
          ? "0 0 0 4px rgba(22,119,255,0.2), 0 8px 20px rgba(22,119,255,0.3)"
          : "0 6px 16px rgba(22,119,255,0.25)",
      }}
    >
      <AimOutlined style={{ fontSize: 20, marginBottom: 4 }} />
      <div style={{ fontSize: 10, opacity: 0.85, letterSpacing: 1.5 }}>TARGET</div>
      <div
        style={{ fontSize: 10, opacity: 0.85, marginTop: 2 }}
      >
        {targets.length} 个 · {strategyLabel(d.strategy)}
      </div>
      <div
        className="num"
        style={{
          fontSize: 12, fontWeight: 600, marginTop: 8,
          wordBreak: "break-all", lineHeight: 1.4,
        }}
      >
        {first ?? "点击设置目标"}
        {targets.length > 1 && (
          <div style={{ fontSize: 10, opacity: 0.8, marginTop: 2 }}>
            +{targets.length - 1} 个
          </div>
        )}
      </div>
      <Handle type="target" position={Position.Left} style={handleStyle} />
    </div>
  );
}

function NodeFrame({
  accent, selected, icon, title, subtitle, children,
  onPlusClick, onDeleteClick,
}: {
  accent: string;
  selected: boolean;
  icon: React.ReactNode;
  title: string;
  subtitle: string;
  children: React.ReactNode;
  onPlusClick?: () => void;
  onDeleteClick?: () => void;
}) {
  return (
    <div
      style={{
        background: "#fff",
        borderRadius: 12,
        width: 240,
        border: selected ? `2px solid ${accent}` : "1px solid #e8e8e8",
        boxShadow: selected
          ? `0 0 0 4px ${accent}22, 0 8px 20px rgba(0,0,0,0.08)`
          : "0 4px 12px rgba(0,0,0,0.05)",
        position: "relative",
        cursor: "pointer",
      }}
    >
      <div
        style={{
          height: 4, background: accent,
          borderTopLeftRadius: 12, borderTopRightRadius: 12,
        }}
      />
      <div style={{ padding: "12px 14px 14px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {icon}
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: "#1f1f1f" }}>{title}</div>
            <div style={{ fontSize: 11, color: "#8c8c8c", marginTop: 1 }}>{subtitle}</div>
          </div>
        </div>
        <div style={{ marginTop: 10 }}>{children}</div>
      </div>

      {onDeleteClick && (
        <Popconfirm
          title="删除此 hop?"
          okText="删除" okType="danger" cancelText="取消"
          onConfirm={onDeleteClick}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            style={{
              position: "absolute", top: -8, right: -8,
              width: 22, height: 22, borderRadius: 11,
              background: "#fff", border: "1px solid #f0f0f0",
              display: "flex", alignItems: "center", justifyContent: "center",
              cursor: "pointer", boxShadow: "0 2px 4px rgba(0,0,0,0.08)",
              fontSize: 11, color: "#8c8c8c",
              zIndex: 5,
            }}
          >
            <CloseOutlined />
          </div>
        </Popconfirm>
      )}

      {onPlusClick && (
        <div
          onClick={(e) => {
            e.stopPropagation();
            onPlusClick();
          }}
          onMouseDown={(e) => e.stopPropagation()}
          style={{
            position: "absolute", right: -16, top: "50%",
            transform: "translateY(-50%)",
            width: 28, height: 28, borderRadius: 14,
            background: accent, color: "#fff",
            display: "flex", alignItems: "center", justifyContent: "center",
            cursor: "pointer",
            boxShadow: `0 4px 10px ${accent}66`,
            fontSize: 12, fontWeight: 600,
            zIndex: 5,
          }}
        >
          <PlusOutlined />
        </div>
      )}
    </div>
  );
}

function NodeChips({
  nodes, accent, withWeight, placeholder,
}: {
  nodes: { id: string; weight: number }[];
  accent: string;
  withWeight?: boolean;
  placeholder: string;
}) {
  if (nodes.length === 0) {
    return (
      <div
        style={{
          border: "1px dashed #d9d9d9", borderRadius: 8,
          padding: "10px 12px", fontSize: 11,
          color: "#bfbfbf", textAlign: "center",
        }}
      >
        {placeholder}
      </div>
    );
  }
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
      {nodes.slice(0, 4).map((n) => (
        <span
          key={n.id}
          className="num"
          style={{
            background: "#fafafa",
            border: `1px solid ${accent}33`,
            color: "#1f1f1f",
            fontSize: 11,
            borderRadius: 4,
            padding: "1px 6px",
          }}
        >
          {n.id}{withWeight && n.weight > 1 ? ` ·${n.weight}` : ""}
        </span>
      ))}
      {nodes.length > 4 && (
        <span style={{
          background: "#fafafa", border: "1px solid #e8e8e8",
          color: "#8c8c8c", fontSize: 11, borderRadius: 4, padding: "1px 6px",
        }}>
          +{nodes.length - 4}
        </span>
      )}
    </div>
  );
}

const nodeTypes = { entry: EntryNode, hop: HopNode, target: TargetNode };

// ── 自定义 Edge（测试结果 chip）─────────────────────────
type EdgeStat = {
  total: number;
  okCount: number;
  minMs: number;
  maxMs: number;
  results: EdgeProbe[];
};

type TestEdgePayload = {
  stat?: EdgeStat;
  testing?: boolean;
  onClick?: (edgeId: string) => void;
};

function statColor(stat?: EdgeStat) {
  if (!stat) return { stroke: "#bfbfbf", chip: "#bfbfbf", bg: "#fafafa" };
  if (stat.okCount === 0)
    return { stroke: "#ff4d4f", chip: "#ff4d4f", bg: "#fff1f0" };
  if (stat.okCount < stat.total)
    return { stroke: "#fa8c16", chip: "#fa8c16", bg: "#fff7e6" };
  if (stat.maxMs < 50) return { stroke: "#52c41a", chip: "#52c41a", bg: "#f6ffed" };
  if (stat.maxMs < 200) return { stroke: "#faad14", chip: "#faad14", bg: "#fffbe6" };
  return { stroke: "#fa8c16", chip: "#fa8c16", bg: "#fff7e6" };
}

function TestEdge(props: EdgeProps) {
  const { id, sourceX, sourceY, targetX, targetY, style } = props;
  const data = (props.data ?? {}) as TestEdgePayload;
  const [path, labelX, labelY] = getBezierPath({
    sourceX, sourceY, targetX, targetY,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
  });
  const c = statColor(data.stat);
  const baseStyle = data.stat
    ? { ...style, stroke: c.stroke, strokeWidth: 2 }
    : (style as React.CSSProperties);

  let chipText = "";
  if (data.testing) chipText = "测试中…";
  else if (data.stat) {
    const s = data.stat;
    if (s.okCount === 0) chipText = `失败 0/${s.total}`;
    else if (s.okCount < s.total) chipText = `${s.minMs}~${s.maxMs}ms · ${s.okCount}/${s.total}`;
    else chipText = s.minMs === s.maxMs ? `${s.minMs}ms` : `${s.minMs}~${s.maxMs}ms`;
  }

  return (
    <>
      <BaseEdge id={id} path={path} style={baseStyle} />
      {chipText && (
        <EdgeLabelRenderer>
          <div
            className="num nodrag nopan"
            onClick={(e) => {
              e.stopPropagation();
              if (!data.testing) data.onClick?.(id);
            }}
            style={{
              position: "absolute",
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
              background: data.testing ? "#e6f4ff" : c.bg,
              border: `1px solid ${data.testing ? "#1677ff44" : c.chip + "55"}`,
              color: data.testing ? "#1677ff" : c.chip,
              padding: "2px 10px",
              borderRadius: 12,
              fontSize: 11,
              fontWeight: 600,
              cursor: data.testing ? "wait" : "pointer",
              pointerEvents: "all",
              boxShadow: "0 2px 6px rgba(0,0,0,0.06)",
              whiteSpace: "nowrap",
            }}
          >
            {chipText}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

const edgeTypes = { test: TestEdge };

// ── 主组件 ───────────────────────────────────────────
export default function TopologyEditor() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { user } = useAuth();
  const { message } = App.useApp();
  const readOnly = !!id && user?.role !== "admin";

  const [allNodes, setAllNodes] = useState<ZNode[]>([]);
  const [form] = Form.useForm();
  const [hops, setHops] = useState<Hop[]>([{ strategy: "weighted", nodes: [] }]);
  const [targets, setTargets] = useState<TargetEndpoint[]>([]);
  const [targetStrategy, setTargetStrategy] = useState<string>("weighted");
  // protocol 走多选：可同时 TCP+UDP。后端规范化为 "tcp" / "udp" / "tcp+udp"
  const [protocols, setProtocols] = useState<string[]>(["tcp"]);
  // selectedHop: 数字 → 中转/入口；"target" → 终点；null 关闭
  const [selectedHop, setSelectedHop] = useState<number | "target" | null>(null);
  const [busy, setBusy] = useState(false);
  // 编辑模式下，数据未加载完前不要构建 RF 节点，否则 target/hops 会按默认长度 1
  // 占位置，等真实数据到达时缓存的 target 位置不会重算 → 和新 hop 重叠堆叠。
  const [pageReady, setPageReady] = useState(!id);
  // #39 编辑模式下保留服务端返回的 forward 快照，用于：
  // - customer 看 read-only 配额状态（quota_exhausted_at_ms / quota_reset_at_ms / 已用流量）
  // - admin 视图与 read-only 视图保持一致
  const [forwardSnapshot, setForwardSnapshot] = useState<Forward | null>(null);
  const isAdmin = user?.role === "admin";

  // 测试相关
  const [testing, setTesting] = useState(false);
  const [edgeStats, setEdgeStats] = useState<Record<string, EdgeStat>>({});
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null);

  useEffect(() => {
    api.get<ZNode[]>("/api/nodes").catch(() => []).then((ns) => setAllNodes(ns as ZNode[]));
    if (id) {
      api.get<Forward[]>("/api/forwards").then((all) => {
        const f = all.find((x) => String(x.id) === id);
        if (f) {
          form.setFieldsValue({
            name: f.name, listen_port: f.listen_port,
            // #39 quota/rate 表单回填（admin only 显示；customer 看 read-only Card）
            quota_in_gb: f.quota_in_bytes ? f.quota_in_bytes / (1024 ** 3) : undefined,
            quota_out_gb: f.quota_out_bytes ? f.quota_out_bytes / (1024 ** 3) : undefined,
            rate_in_mbps: f.rate_in_bps ? f.rate_in_bps / (1024 ** 2) : undefined,
            rate_out_mbps: f.rate_out_bps ? f.rate_out_bps / (1024 ** 2) : undefined,
            quota_reset: f.quota_reset ?? "none",
            link_encryption: f.link_encryption ?? "tls",
            path_mode: f.path_mode ?? "auto",
          });
          const parts = (f.protocol || "tcp")
            .split("+").map((x) => x.trim().toLowerCase()).filter(Boolean);
          setProtocols(parts.length ? parts : ["tcp"]);
          setHops(f.hops);
          setTargets(f.targets ?? []);
          setTargetStrategy(f.target_strategy || "weighted");
          setForwardSnapshot(f);
        }
        setPageReady(true);
      }).catch(() => setPageReady(true));
    } else {
      form.setFieldsValue({ listen_port: 10080, quota_reset: "none" });
    }
  }, [id, form]);

  const listenPort = Form.useWatch("listen_port", form);

  // target 操作
  const addTarget = useCallback(() =>
    setTargets((ts) => [...ts, { addr: "", weight: 1 }]), []);
  const removeTarget = useCallback((i: number) =>
    setTargets((ts) => ts.filter((_, x) => x !== i)), []);
  const setTargetAddr = useCallback((i: number, addr: string) =>
    setTargets((ts) => ts.map((t, x) => (x === i ? { ...t, addr } : t))), []);
  const setTargetWeight = useCallback((i: number, w: number) =>
    setTargets((ts) => ts.map((t, x) => (x === i ? { ...t, weight: w } : t))), []);

  // ── hop ops（全部用函数式更新，避免 stale closure）─────
  const insertHopAfter = useCallback((hi: number) => {
    setHops((hs) => [
      ...hs.slice(0, hi + 1),
      { strategy: "weighted", nodes: [] },
      ...hs.slice(hi + 1),
    ]);
    setSelectedHop(hi + 1);
  }, []);

  const removeHop = useCallback((hi: number) => {
    if (hi === 0) return; // 入口不可删
    setHops((hs) => (hs.length <= 1 ? hs : hs.filter((_, x) => x !== hi)));
    setSelectedHop((s) => {
      if (s === null) return null;
      if (s === "target") return s;
      if (s === hi) return null;
      return s > hi ? s - 1 : s;
    });
  }, []);

  const setStrategy = useCallback((hi: number, s: string) =>
    setHops((hs) => hs.map((h, x) => (x === hi ? { ...h, strategy: s } : h))), []);

  const addNode = useCallback((hi: number, nid: string) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === hi && !h.nodes.some((n) => n.id === nid)
          ? { ...h, nodes: [...h.nodes, { id: nid, weight: 1 }] }
          : h
      )
    ), []);

  const removeNode = useCallback((hi: number, ni: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === hi ? { ...h, nodes: h.nodes.filter((_, y) => y !== ni) } : h
      )
    ), []);

  const setWeight = useCallback((hi: number, ni: number, w: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === hi
          ? { ...h, nodes: h.nodes.map((n, y) => (y === ni ? { ...n, weight: w } : n)) }
          : h
      )
    ), []);

  // ── RF nodes/edges ───────────────────────────────────
  const [rfNodes, setRfNodes, onNodesChange] = useNodesState<RFNode>([]);
  const [rfEdges, setRfEdges, onEdgesChange] = useEdgesState<RFEdge>([]);

  useEffect(() => {
    if (!pageReady) return; // 编辑模式：数据未到，先不画图
    setRfNodes((current) => {
      const posMap = new Map(current.map((n) => [n.id, n.position]));
      const next: RFNode[] = [];
      hops.forEach((h, hi) => {
        next.push({
          id: `hop-${hi}`,
          type: hi === 0 ? "entry" : "hop",
          position: posMap.get(`hop-${hi}`) ?? { x: hi * 320, y: 120 },
          data: {
            hi, hop: h, listenPort, readOnly,
            onPlusClick: insertHopAfter,
            onDeleteClick: removeHop,
          } as any,
          deletable: false,
          draggable: !readOnly,
        });
      });
      next.push({
        id: "target",
        type: "target",
        position: posMap.get("target") ?? { x: hops.length * 320, y: 120 },
        data: { targets, strategy: targetStrategy, readOnly } as any,
        deletable: false,
        draggable: !readOnly,
      });
      return next;
    });
  }, [hops, targets, targetStrategy, listenPort, readOnly, insertHopAfter, removeHop, setRfNodes, pageReady]);

  const onEdgeChipClick = useCallback((eid: string) => setSelectedEdge(eid), []);

  useEffect(() => {
    const e: RFEdge[] = [];
    const mk = (id: string, source: string, targetId: string, fallbackStroke: string): RFEdge => ({
      id, source, target: targetId,
      type: "test",
      animated: !edgeStats[id],
      style: { stroke: fallbackStroke, strokeWidth: 2 },
      data: {
        stat: edgeStats[id],
        testing,
        onClick: onEdgeChipClick,
      } as TestEdgePayload as any,
    });
    for (let i = 0; i < hops.length - 1; i++) {
      e.push(mk(`e-${i}`, `hop-${i}`, `hop-${i + 1}`, "#bfbfbf"));
    }
    if (hops.length > 0) {
      e.push(mk(`e-target`, `hop-${hops.length - 1}`, "target", "#1677ff"));
    }
    setRfEdges(e);
  }, [hops, edgeStats, testing, onEdgeChipClick, setRfEdges]);

  // ── 链路测试 ─────────────────────────────────────────
  function aggregate(hopsLocal: Hop[], results: EdgeProbe[]): Record<string, EdgeStat> {
    const out: Record<string, EdgeStat> = {};
    const stat = (rs: EdgeProbe[]): EdgeStat => {
      const ok = rs.filter((r) => r.ok);
      const lats = ok.map((r) => r.latency_ms);
      return {
        total: rs.length,
        okCount: ok.length,
        minMs: lats.length ? Math.min(...lats) : 0,
        maxMs: lats.length ? Math.max(...lats) : 0,
        results: rs,
      };
    };
    for (let i = 0; i < hopsLocal.length - 1; i++) {
      const fromIds = new Set(hopsLocal[i].nodes.map((n) => n.id));
      const toIds = new Set(hopsLocal[i + 1].nodes.map((n) => n.id));
      out[`e-${i}`] = stat(results.filter(
        (r) => fromIds.has(r.from_node) && r.to_node && toIds.has(r.to_node)
      ));
    }
    if (hopsLocal.length > 0) {
      const lastIds = new Set(hopsLocal[hopsLocal.length - 1].nodes.map((n) => n.id));
      out[`e-target`] = stat(results.filter(
        (r) => r.to_node === null && lastIds.has(r.from_node)
      ));
    }
    return out;
  }

  async function runTest() {
    try {
      const values = await form.validateFields();
      if (hops.some((h) => h.nodes.length === 0)) {
        message.error("每个 hop 至少需要一个节点才能测试");
        return;
      }
      const cleanTargets = targets.filter((t) => t.addr.trim() !== "");
      if (cleanTargets.length === 0) {
        message.error("至少需要 1 个目标地址才能测试");
        return;
      }
      if (protocols.length === 0) {
        message.error("至少需要选择一个协议（TCP/UDP）");
        return;
      }
      setTesting(true);
      setEdgeStats({});
      const resp = await api.post<TestResponse>("/api/forwards/test", {
        ...values, hops,
        protocol: protocols.join("+"),
        targets: cleanTargets,
        target_strategy: targetStrategy,
      });
      setEdgeStats(aggregate(hops, resp.results));
    } catch (e: any) {
      if (e?.errorFields) return;
      message.error(e.message);
    } finally {
      setTesting(false);
    }
  }

  // ── save ─────────────────────────────────────────────
  async function save() {
    try {
      const values = await form.validateFields();
      if (hops.some((h) => h.nodes.length === 0)) {
        message.error("每个 hop 至少需要一个节点");
        return;
      }
      const cleanTargets = targets.filter((t) => t.addr.trim() !== "");
      if (cleanTargets.length === 0) {
        message.error("至少需要 1 个目标地址（点终点节点添加）");
        return;
      }
      if (protocols.length === 0) {
        message.error("至少需要选择一个协议（TCP/UDP）");
        return;
      }
      setBusy(true);
      // #39 quota/rate 表单显示用 GB / MB/s，发送用 bytes / bytes-per-second。
      // 非 admin 不发这些字段（后端会自动忽略，但 client side 也明确不传减少误会）。
      const quotaFields = isAdmin ? {
        quota_in_bytes: values.quota_in_gb ? Math.round(values.quota_in_gb * 1024 ** 3) : null,
        quota_out_bytes: values.quota_out_gb ? Math.round(values.quota_out_gb * 1024 ** 3) : null,
        rate_in_bps: values.rate_in_mbps ? Math.round(values.rate_in_mbps * 1024 ** 2) : null,
        rate_out_bps: values.rate_out_mbps ? Math.round(values.rate_out_mbps * 1024 ** 2) : null,
        quota_reset: values.quota_reset && values.quota_reset !== "none" ? values.quota_reset : null,
        // #27 链路加密：admin 才发；customer 不传由 master 端兜底 admin-gate
        link_encryption: values.link_encryption === "plain" ? "plain" : "tls",
        // M4.2 fast path 模式：admin 才发
        path_mode: ["fast", "slow"].includes(values.path_mode) ? values.path_mode : "auto",
      } : {};
      const {
        quota_in_gb: _qig, quota_out_gb: _qog,
        rate_in_mbps: _rim, rate_out_mbps: _rom,
        quota_reset: _qr,
        link_encryption: _le,
        path_mode: _pm,
        ...basicValues
      } = values;
      const payload = {
        ...basicValues, hops,
        protocol: protocols.join("+"),
        targets: cleanTargets,
        target_strategy: targetStrategy,
        ...quotaFields,
      };
      if (id) await api.put(`/api/forwards/${id}`, payload);
      else await api.post("/api/forwards", payload);
      message.success(id ? "已保存修改" : "已创建");
      navigate("/forwards");
    } catch (e: any) {
      if (e?.errorFields) return;
      message.error(e.message);
    } finally {
      setBusy(false);
    }
  }

  // ── render ───────────────────────────────────────────
  const isTargetSelected = selectedHop === "target";
  const selectedHopData =
    typeof selectedHop === "number" ? hops[selectedHop] : null;
  const isEntrySelected = selectedHop === 0;

  return (
    <div style={{ maxWidth: 1400, margin: "0 auto" }}>
      <Button
        type="link" icon={<ArrowLeftOutlined />}
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
            点节点弹出抽屉配置 · 点节点右侧 <Tag color="blue" style={{ margin: "0 2px" }}>+</Tag> 追加下一步
          </Text>
        </div>
        {!readOnly && (
          <Space>
            <Button
              size="large" icon={<ThunderboltOutlined />}
              loading={testing} onClick={runTest}
            >
              测试链路
            </Button>
            <Button
              type="primary" size="large" icon={<SaveOutlined />}
              loading={busy} onClick={save}
            >
              {id ? "保存修改" : "创建转发"}
            </Button>
          </Space>
        )}
      </div>

      <Form form={form} layout="vertical" disabled={readOnly} requiredMark={false}>
        <Card style={{ marginBottom: 16 }} bodyStyle={{ padding: "16px 20px" }}>
          <div style={{
            display: "grid",
            gridTemplateColumns: "1.8fr 1fr 0.8fr",
            gap: 16, alignItems: "end",
          }}>
            <Form.Item
              name="name" label="转发名称" style={{ marginBottom: 0 }}
              rules={[{ required: true, message: "请输入名称" }]}
            >
              <Input placeholder="比如：上海到日本游戏服" />
            </Form.Item>
            <Form.Item
              name="listen_port" label="监听端口" style={{ marginBottom: 0 }}
              rules={[{ required: true, message: "请输入端口" }]}
            >
              <InputNumber min={1} max={65535} style={{ width: "100%" }} className="num" />
            </Form.Item>
            <Form.Item label="协议" style={{ marginBottom: 0 }}>
              <Checkbox.Group
                value={protocols}
                onChange={(v) => setProtocols(v as string[])}
                options={[{ label: "TCP", value: "tcp" }, { label: "UDP", value: "udp" }]}
                disabled={readOnly}
              />
            </Form.Item>
          </div>
          <Text type="secondary" style={{ fontSize: 11, marginTop: 8, display: "block" }}>
            目标地址在拓扑右侧的「TARGET」节点里编辑（支持多目标 + LB 策略）
          </Text>
        </Card>

        <Card
          title="拓扑链路"
          style={{ marginBottom: 32 }}
          bodyStyle={{ padding: 0, background: "#f5f7fa" }}
        >
          <div style={{ height: 560, borderRadius: "0 0 10px 10px", overflow: "hidden" }}>
            <ReactFlow
              nodes={rfNodes}
              edges={rfEdges}
              nodeTypes={nodeTypes}
              edgeTypes={edgeTypes}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onNodeClick={(_, node) => {
                if (node.id === "target") {
                  setSelectedHop("target");
                  return;
                }
                const m = node.id.match(/^hop-(\d+)$/);
                if (m) setSelectedHop(parseInt(m[1]));
              }}
              fitView
              fitViewOptions={{ padding: 0.25 }}
              minZoom={0.4}
              maxZoom={1.6}
              nodesConnectable={false}
              elementsSelectable
              proOptions={{ hideAttribution: true }}
            >
              <Background color="#d0d6dd" gap={22} size={1.4} />
              <Controls showInteractive={false} position="bottom-right" />
            </ReactFlow>
          </div>
        </Card>

        <QuotaSection
          isAdmin={isAdmin}
          readOnly={readOnly}
          snapshot={forwardSnapshot}
        />

        <LinkEncryptionSection
          isAdmin={isAdmin}
          readOnly={readOnly}
          protocols={protocols}
          snapshot={forwardSnapshot}
        />
      </Form>

      {/* 配置抽屉 */}
      <Drawer
        title={
          isTargetSelected
            ? "目标地址（多 target + LB）"
            : isEntrySelected
              ? "客户端入口"
              : typeof selectedHop === "number"
                ? `中转 ${selectedHop}`
                : ""
        }
        placement="right"
        open={selectedHop !== null}
        onClose={() => setSelectedHop(null)}
        width={460}
        extra={
          typeof selectedHop === "number" && !isEntrySelected && !readOnly ? (
            <Popconfirm
              title="删除此 hop?"
              okText="删除" okType="danger" cancelText="取消"
              onConfirm={() => removeHop(selectedHop as number)}
            >
              <Button danger icon={<DeleteOutlined />}>删除</Button>
            </Popconfirm>
          ) : null
        }
      >
        {isTargetSelected && (
          <TargetConfigPanel
            targets={targets}
            strategy={targetStrategy}
            readOnly={readOnly}
            onStrategy={setTargetStrategy}
            onAdd={addTarget}
            onRemove={removeTarget}
            onAddr={setTargetAddr}
            onWeight={setTargetWeight}
          />
        )}
        {selectedHopData && (
          <HopConfigPanel
            hop={selectedHopData}
            allNodes={allNodes}
            readOnly={readOnly}
            isEntry={isEntrySelected}
            listenPort={listenPort}
            onStrategy={(s) => setStrategy(selectedHop as number, s)}
            onAddNode={(nid) => addNode(selectedHop as number, nid)}
            onRemoveNode={(ni) => removeNode(selectedHop as number, ni)}
            onSetWeight={(ni, w) => setWeight(selectedHop as number, ni, w)}
          />
        )}
      </Drawer>

      {/* 边探测结果详情 */}
      <Drawer
        title={
          selectedEdge === "e-target"
            ? "出口 → 目标 · 探测详情"
            : selectedEdge?.startsWith("e-")
              ? `第 ${parseInt(selectedEdge.slice(2)) + 1} 跳 → 第 ${parseInt(selectedEdge.slice(2)) + 2} 跳 · 探测详情`
              : ""
        }
        placement="right" width={520}
        open={!!selectedEdge}
        onClose={() => setSelectedEdge(null)}
      >
        <EdgeProbeDetails stat={selectedEdge ? edgeStats[selectedEdge] : undefined} />
      </Drawer>
    </div>
  );
}

function EdgeProbeDetails({ stat }: { stat?: EdgeStat }) {
  if (!stat || stat.total === 0) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有探测结果" />;
  }
  const okPct = stat.total > 0 ? (stat.okCount / stat.total) * 100 : 0;
  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      <div style={{
        display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 12,
      }}>
        <Stat label="可达对" value={`${stat.okCount}/${stat.total}`}
          color={okPct === 100 ? "#52c41a" : okPct === 0 ? "#ff4d4f" : "#fa8c16"} />
        <Stat label="最低延迟" value={stat.okCount ? `${stat.minMs} ms` : "—"} color="#1677ff" />
        <Stat label="最高延迟" value={stat.okCount ? `${stat.maxMs} ms` : "—"} color="#722ed1" />
      </div>
      <Space direction="vertical" size={6} style={{ width: "100%" }}>
        {stat.results.map((r, i) => (
          <div
            key={i}
            style={{
              display: "flex", alignItems: "center", gap: 8,
              background: "#fafafa", border: "1px solid #f0f0f0",
              borderRadius: 8, padding: "8px 12px", fontSize: 12,
            }}
          >
            {r.ok
              ? <CheckCircleFilled style={{ color: "#52c41a" }} />
              : <CloseCircleFilled style={{ color: "#ff4d4f" }} />}
            <Text className="num" strong>{r.from_node}</Text>
            <Text type="secondary">→</Text>
            <Text className="num" strong>{r.to_node ?? "target"}</Text>
            <Text type="secondary" className="num" style={{ fontSize: 11 }}>
              ({r.to_addr})
            </Text>
            <div style={{ flex: 1 }} />
            {r.ok ? (
              <Text strong className="num" style={{ color: "#1677ff" }}>{r.latency_ms} ms</Text>
            ) : (
              <Text type="danger" style={{ fontSize: 11, maxWidth: 200, textAlign: "right" }}>
                {r.error || "失败"}
              </Text>
            )}
          </div>
        ))}
      </Space>
    </Space>
  );
}

function Stat({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div style={{
      background: "#fafafa", border: "1px solid #f0f0f0",
      borderRadius: 8, padding: "10px 12px",
    }}>
      <div style={{ fontSize: 11, color: "#8c8c8c" }}>{label}</div>
      <div className="num" style={{
        fontSize: 18, fontWeight: 600, color, marginTop: 2,
      }}>
        {value}
      </div>
    </div>
  );
}

function TargetConfigPanel(props: {
  targets: TargetEndpoint[];
  strategy: string;
  readOnly: boolean;
  onStrategy: (s: string) => void;
  onAdd: () => void;
  onRemove: (i: number) => void;
  onAddr: (i: number, addr: string) => void;
  onWeight: (i: number, w: number) => void;
}) {
  const { targets, strategy, readOnly, onStrategy, onAdd, onRemove, onAddr, onWeight } = props;
  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      <div style={{
        background: "#e6f4ff", border: "1px solid #91caff",
        padding: "10px 12px", borderRadius: 8, fontSize: 12, lineHeight: 1.6,
      }}>
        <AimOutlined style={{ color: "#1677ff", marginRight: 6 }} />
        多个目标按策略分流；任一目标不可达时自动 failover 到下一个。
      </div>

      <div>
        <div style={{ fontSize: 12, color: "#8c8c8c", marginBottom: 6 }}>负载均衡策略</div>
        <Select
          value={strategy}
          onChange={onStrategy}
          options={PATH_STRATEGIES}
          disabled={readOnly}
          style={{ width: "100%" }}
        />
      </div>

      <div>
        <div style={{
          fontSize: 12, color: "#8c8c8c", marginBottom: 8,
          display: "flex", justifyContent: "space-between",
        }}>
          <span>目标列表（{targets.length}）</span>
          <span style={{ fontSize: 11 }}>host:port</span>
        </div>
        {targets.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有目标地址" />
        ) : (
          <Space direction="vertical" size={8} style={{ width: "100%" }}>
            {targets.map((t, i) => (
              <div
                key={i}
                style={{
                  display: "flex", alignItems: "center", gap: 8,
                  background: "#fafafa", borderRadius: 8,
                  padding: "8px 10px", border: "1px solid #f0f0f0",
                }}
              >
                <Input
                  value={t.addr}
                  placeholder="1.2.3.4:443"
                  disabled={readOnly}
                  onChange={(e) => onAddr(i, e.target.value)}
                  style={{ flex: 1 }}
                  className="num"
                />
                <Space size={4}>
                  <Text type="secondary" style={{ fontSize: 11 }}>w</Text>
                  <InputNumber
                    size="small" min={1} max={1000}
                    value={t.weight}
                    disabled={readOnly}
                    onChange={(v) => onWeight(i, Number(v) || 1)}
                    style={{ width: 64 }}
                    className="num"
                  />
                </Space>
                {!readOnly && (
                  <Button
                    type="text" size="small" danger
                    icon={<CloseOutlined />}
                    onClick={() => onRemove(i)}
                  />
                )}
              </div>
            ))}
          </Space>
        )}
        {!readOnly && (
          <Button
            type="dashed" block icon={<PlusOutlined />}
            onClick={onAdd}
            style={{ marginTop: 8 }}
          >
            添加目标
          </Button>
        )}
      </div>
    </Space>
  );
}

function HopConfigPanel(props: {
  hop: Hop;
  allNodes: ZNode[];
  readOnly: boolean;
  isEntry: boolean;
  listenPort?: number;
  onStrategy: (s: string) => void;
  onAddNode: (nid: string) => void;
  onRemoveNode: (ni: number) => void;
  onSetWeight: (ni: number, w: number) => void;
}) {
  const {
    hop, allNodes, readOnly, isEntry, listenPort,
    onStrategy, onAddNode, onRemoveNode, onSetWeight,
  } = props;
  const available = allNodes.filter((n) => !hop.nodes.some((x) => x.id === n.id));

  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      {isEntry ? (
        <div style={{
          background: "#e6f4ff", border: "1px solid #91caff",
          padding: "10px 12px", borderRadius: 8, fontSize: 12,
          lineHeight: 1.6,
        }}>
          <ThunderboltFilled style={{ color: "#1677ff", marginRight: 6 }} />
          客户端直接连接任一入口节点 IP 的 <Text strong className="num">:{listenPort ?? "—"}</Text>。
          入口的选择在客户端那一端发生，本段无负载均衡策略。
        </div>
      ) : (
        <div>
          <div style={{ fontSize: 12, color: "#8c8c8c", marginBottom: 6 }}>负载均衡策略</div>
          <Select
            value={hop.strategy}
            onChange={onStrategy}
            options={PATH_STRATEGIES}
            disabled={readOnly}
            style={{ width: "100%" }}
          />
        </div>
      )}

      <div>
        <div style={{
          fontSize: 12, color: "#8c8c8c", marginBottom: 8,
          display: "flex", justifyContent: "space-between",
        }}>
          <span>节点列表（{hop.nodes.length}）</span>
          <span style={{ fontSize: 11 }}>{isEntry ? "客户端任选" : "按策略分流"}</span>
        </div>

        {hop.nodes.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有节点" />
        ) : (
          <Space direction="vertical" size={8} style={{ width: "100%" }}>
            {hop.nodes.map((n, ni) => {
              const meta = allNodes.find((x) => x.id === n.id);
              return (
                <div
                  key={n.id}
                  style={{
                    display: "flex", alignItems: "center", gap: 10,
                    background: "#fafafa", borderRadius: 8,
                    padding: "8px 12px", border: "1px solid #f0f0f0",
                  }}
                >
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <Text strong className="num" style={{ fontSize: 13 }}>{n.id}</Text>
                    {meta?.name && (
                      <div style={{ fontSize: 11, color: "#8c8c8c" }}>{meta.name}</div>
                    )}
                  </div>
                  {!isEntry && (
                    <Space size={4}>
                      <Text type="secondary" style={{ fontSize: 11 }}>w</Text>
                      <InputNumber
                        size="small" min={1} max={1000}
                        value={n.weight}
                        disabled={readOnly}
                        onChange={(v) => onSetWeight(ni, Number(v) || 1)}
                        style={{ width: 64 }}
                        className="num"
                      />
                    </Space>
                  )}
                  {!readOnly && (
                    <Button
                      type="text" size="small" danger
                      icon={<CloseOutlined />}
                      onClick={() => onRemoveNode(ni)}
                    />
                  )}
                </div>
              );
            })}
          </Space>
        )}

        {!readOnly && available.length > 0 && (
          <Select
            placeholder="+ 添加节点"
            value={undefined as any}
            onChange={(v) => v && onAddNode(v as string)}
            options={available.map((n) => ({ value: n.id, label: `${n.id} · ${n.name}` }))}
            style={{ width: "100%", marginTop: 8 }}
          />
        )}
      </div>
    </Space>
  );
}

// ── #39 流量限制配置区 ─────────────────────────────────
// admin: 表单输入；customer: 仅显示当前状态（已用 / 上限 / 重置倒计时 / 触达状态）。
// 单位约定：UI 用 GB / MB·s⁻¹，发送时由 save() 换成 bytes / bytes·s⁻¹。
function QuotaSection({
  isAdmin, readOnly, snapshot,
}: {
  isAdmin: boolean;
  readOnly: boolean;
  snapshot: Forward | null;
}) {
  const exhausted = snapshot?.quota_exhausted_at_ms != null;
  return (
    <Card
      title="流量限制（可选）"
      size="small"
      style={{ marginBottom: 32 }}
    >
      {!isAdmin && (
        <Alert
          type="info"
          showIcon
          message="仅管理员可配置流量上限"
          description={snapshot
            ? "下方显示该转发当前的限额状态。如需调整请联系管理员。"
            : "新转发由管理员初始化后才会有限额。"}
          style={{ marginBottom: 16 }}
        />
      )}
      {exhausted && (
        <Alert
          type="warning"
          showIcon
          message="已触达流量上限，转发被软停"
          description={snapshot?.quota_reset_at_ms
            ? `下次重置：${new Date(snapshot.quota_reset_at_ms).toLocaleString()}（UTC）`
            : "重置策略为「永不」—— 需要管理员手动重置或上调配额。"}
          style={{ marginBottom: 16 }}
        />
      )}

      {snapshot && (
        <div style={{ marginBottom: 16, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
          <QuotaUsageBar
            label="上传（客户端→target）"
            used={snapshot.bytes_in ?? 0}
            limit={snapshot.quota_in_bytes ?? null}
          />
          <QuotaUsageBar
            label="下载（target→客户端）"
            used={snapshot.bytes_out ?? 0}
            limit={snapshot.quota_out_bytes ?? null}
          />
        </div>
      )}

      <div style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr 1fr",
        gap: 16, alignItems: "end",
      }}>
        <Form.Item
          name="quota_in_gb"
          label="上传 quota (GB)"
          tooltip="累计上传到达上限后软停转发；空 = 不限"
          style={{ marginBottom: 0 }}
        >
          <InputNumber
            min={0} step={1} precision={2}
            placeholder="不限"
            style={{ width: "100%" }} className="num"
            disabled={readOnly || !isAdmin}
          />
        </Form.Item>
        <Form.Item
          name="quota_out_gb"
          label="下载 quota (GB)"
          tooltip="累计下载到达上限后软停转发；空 = 不限"
          style={{ marginBottom: 0 }}
        >
          <InputNumber
            min={0} step={1} precision={2}
            placeholder="不限"
            style={{ width: "100%" }} className="num"
            disabled={readOnly || !isAdmin}
          />
        </Form.Item>
        <Form.Item
          name="quota_reset"
          label="重置周期"
          tooltip="到点自动清零累计 + 恢复软停；none = 仅手动重置"
          style={{ marginBottom: 0 }}
        >
          <Select
            options={[
              { value: "none", label: "永不重置（手动）" },
              { value: "daily", label: "每日 UTC 00:00" },
              { value: "monthly", label: "每月 1 号 UTC 00:00" },
            ]}
            disabled={readOnly || !isAdmin}
          />
        </Form.Item>
        <Form.Item
          name="rate_in_mbps"
          label="上传带宽 (MB/s)"
          tooltip="瞬时带宽上限，token bucket；空 = 不限"
          style={{ marginBottom: 0 }}
        >
          <InputNumber
            min={0} step={1} precision={2}
            placeholder="不限"
            style={{ width: "100%" }} className="num"
            disabled={readOnly || !isAdmin}
          />
        </Form.Item>
        <Form.Item
          name="rate_out_mbps"
          label="下载带宽 (MB/s)"
          tooltip="瞬时带宽上限，token bucket；空 = 不限"
          style={{ marginBottom: 0 }}
        >
          <InputNumber
            min={0} step={1} precision={2}
            placeholder="不限"
            style={{ width: "100%" }} className="num"
            disabled={readOnly || !isAdmin}
          />
        </Form.Item>
        {snapshot?.quota_reset_at_ms && !exhausted && (
          <div style={{ alignSelf: "end", paddingBottom: 4 }}>
            <Text type="secondary" style={{ fontSize: 12 }}>
              下次重置：{new Date(snapshot.quota_reset_at_ms).toLocaleString()}
            </Text>
          </div>
        )}
      </div>
    </Card>
  );
}

// ── #27 节点间链路加密开关 ─────────────────────────────────
// 'tls'（默认）= 节点间走 mTLS；'plain' = 节点间 TCP 不裹 TLS（同机房 / 内网信任）。
// UDP 链路受 QUIC 协议层强制 TLS，选 plain 仅对 TCP 生效，UI 给提示。
function LinkEncryptionSection({
  isAdmin, readOnly, protocols, snapshot,
}: {
  isAdmin: boolean;
  readOnly: boolean;
  protocols: string[];
  snapshot: Forward | null;
}) {
  const hasUdp = protocols.includes("udp");
  const current = snapshot?.link_encryption ?? "tls";
  return (
    <Card title="链路加密（节点之间）" size="small" style={{ marginBottom: 32 }}>
      {!isAdmin && (
        <Alert
          type="info"
          showIcon
          message="仅管理员可调整链路加密模式"
          description={`当前模式：${current === "plain" ? "明文（节点间 TCP 不裹 TLS）" : "TLS（默认）"}`}
          style={{ marginBottom: 16 }}
        />
      )}
      <Form.Item
        name="link_encryption"
        label="节点↔节点 传输"
        tooltip="仅影响多跳之间的 raw_tunnel TCP 链路；客户端↔入口、出口↔target 原协议不变；master↔node 控制面永远 mTLS"
        style={{ marginBottom: 8 }}
      >
        <Select
          options={[
            { value: "tls", label: "自动 — TLS 加密（默认，推荐公网）" },
            { value: "plain", label: "明文 — 同机房 / 信任内网（节点间零 TLS 开销）" },
          ]}
          disabled={readOnly || !isAdmin}
          style={{ maxWidth: 480 }}
        />
      </Form.Item>
      {hasUdp && (
        <Alert
          type="warning"
          showIcon
          message="UDP 链路仍走 QUIC + TLS"
          description="QUIC 协议层强制要求 TLS，明文模式仅对 TCP 跳生效。UDP forward 选 plain 不报错但底层照常加密。"
          style={{ marginTop: 8 }}
        />
      )}

      {/* M4.2 fast path 路径模式 */}
      <Form.Item
        name="path_mode"
        label="转发路径"
        tooltip="auto：单跳 + 明文 + 节点支持 nftables → 走内核 fast path；其它情况 slow path。fast：强制尝试内核（失败回退 slow）。slow：强制用户态 tokio 转发。"
        style={{ marginBottom: 0, marginTop: 16 }}
      >
        <Select
          options={[
            { value: "auto", label: "自动 — 满足条件走内核 fast path" },
            { value: "fast", label: "强制 fast — 内核 nftables DNAT（失败自动回退 slow）" },
            { value: "slow", label: "强制 slow — 用户态 tokio 转发（保留 session 历史/双向流量统计）" },
          ]}
          disabled={readOnly || !isAdmin}
          style={{ maxWidth: 480 }}
        />
      </Form.Item>
      <Alert
        type="info"
        showIcon
        message="fast path 限制"
        description="只对单跳 + 单 target + 非 TLS 加密的 TCP/UDP 生效；多跳 / 多 target / TLS 永远 slow。fast 模式下 #36 会话历史不可用、流量统计仅入向（出向 V2）。"
        style={{ marginTop: 8 }}
      />
    </Card>
  );
}

function QuotaUsageBar({ label, used, limit }: { label: string; used: number; limit: number | null }) {
  if (!limit || limit <= 0) {
    return (
      <div>
        <Text type="secondary" style={{ fontSize: 12 }}>{label}</Text>
        <div style={{ marginTop: 4 }}>
          <Text className="num" style={{ fontSize: 14 }}>{formatBytesGb(used)}</Text>
          <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>· 无上限</Text>
        </div>
      </div>
    );
  }
  const pct = Math.min(100, Math.round((used / limit) * 100));
  const status: "normal" | "exception" | "active" =
    pct >= 100 ? "exception" : pct >= 80 ? "active" : "normal";
  return (
    <div>
      <Text type="secondary" style={{ fontSize: 12 }}>{label}</Text>
      <Progress
        percent={pct}
        status={status}
        format={() => `${formatBytesGb(used)} / ${formatBytesGb(limit)}`}
        size="small"
      />
    </div>
  );
}

function formatBytesGb(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 ** 2) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1024 ** 3) return `${(b / 1024 ** 2).toFixed(1)} MB`;
  return `${(b / 1024 ** 3).toFixed(2)} GB`;
}
