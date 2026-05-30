import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
  ReactFlow,
  Background,
  Controls,
  type Node as RFNode,
  type Edge as RFEdge,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { api, type Forward, type Hop, type Node } from "../lib/api";
import { useAuth } from "../lib/auth";

const STRATEGIES = [
  { v: "weighted", label: "加权轮询" },
  { v: "source_hash", label: "会话保持(源IP哈希)" },
  { v: "least_conn", label: "最小连接数" },
  { v: "latency", label: "延迟最优" },
];

export default function TopologyEditor() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { user } = useAuth();
  const readOnly = !!id && user?.role !== "admin"; // 客户编辑暂不开放：只读查看
  const [nodes, setNodes] = useState<Node[]>([]);
  const [name, setName] = useState("");
  const [listen, setListen] = useState(10080);
  const [protocol, setProtocol] = useState("tcp");
  const [target, setTarget] = useState("");
  const [hops, setHops] = useState<Hop[]>([{ strategy: "weighted", nodes: [] }]);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.get<Node[]>("/api/nodes").catch(() => [] as Node[]).then(setNodes as any);
    if (id) {
      api.get<Forward[]>("/api/forwards").then((all) => {
        const f = all.find((x) => String(x.id) === id);
        if (f) {
          setName(f.name);
          setListen(f.listen_port);
          setProtocol(f.protocol);
          setTarget(f.target);
          setHops(f.hops);
        }
      });
    }
  }, [id]);

  function addHop() {
    setHops([...hops, { strategy: "weighted", nodes: [] }]);
  }
  function rmHop(i: number) {
    setHops(hops.filter((_, x) => x !== i));
  }
  function setHopStrategy(i: number, s: string) {
    setHops(hops.map((h, x) => (x === i ? { ...h, strategy: s } : h)));
  }
  function addNode(i: number, nodeId: string) {
    if (!nodeId) return;
    setHops(
      hops.map((h, x) =>
        x === i && !h.nodes.some((n) => n.id === nodeId)
          ? { ...h, nodes: [...h.nodes, { id: nodeId, weight: 1 }] }
          : h
      )
    );
  }
  function setNodeWeight(hi: number, ni: number, w: number) {
    setHops(
      hops.map((h, x) =>
        x === hi ? { ...h, nodes: h.nodes.map((n, y) => (y === ni ? { ...n, weight: w } : n)) } : h
      )
    );
  }
  function rmNode(hi: number, ni: number) {
    setHops(hops.map((h, x) => (x === hi ? { ...h, nodes: h.nodes.filter((_, y) => y !== ni) } : h)));
  }

  // React Flow 拓扑可视化（每跳一列，组内节点纵向堆叠）
  const { rfNodes, rfEdges } = useMemo(() => {
    const rn: RFNode[] = [];
    const re: RFEdge[] = [];
    hops.forEach((h, hi) => {
      if (h.nodes.length === 0) {
        rn.push({
          id: `h${hi}-empty`,
          data: { label: `第 ${hi + 1} 跳 (空)` },
          position: { x: hi * 220, y: 80 },
          style: {
            background: "#181d25",
            border: "1px dashed #2a3340",
            color: "#5b6470",
            borderRadius: 8,
            fontSize: 12,
          },
        });
      }
      h.nodes.forEach((n, ni) => {
        const id = `h${hi}-${n.id}`;
        rn.push({
          id,
          data: {
            label: (
              <div style={{ fontSize: 11 }}>
                <div style={{ fontWeight: 600 }}>{n.id}</div>
                <div style={{ color: "#8a93a3", fontSize: 10 }}>
                  w={n.weight}
                </div>
              </div>
            ) as any,
          },
          position: { x: hi * 220, y: ni * 70 + 20 },
          style: {
            background: "#12161c",
            border: "1px solid #5ad6ff",
            color: "#e6e9ee",
            borderRadius: 8,
            padding: 6,
            width: 130,
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
          re.push({ id: `${a}-${b}`, source: a, target: b, style: { stroke: "#2a3340" } });
    }
    // target 终点
    if (target && hops.length > 0) {
      const last = hops[hops.length - 1];
      rn.push({
        id: "target",
        data: { label: `→ ${target}` },
        position: { x: hops.length * 220, y: 80 },
        style: {
          background: "#181d25",
          border: "1px solid #7cf3a0",
          color: "#7cf3a0",
          borderRadius: 8,
          fontSize: 11,
          padding: 8,
        },
      });
      const sources = last.nodes.length
        ? last.nodes.map((n) => `h${hops.length - 1}-${n.id}`)
        : [`h${hops.length - 1}-empty`];
      for (const s of sources)
        re.push({ id: `${s}-target`, source: s, target: "target", style: { stroke: "#7cf3a0" } });
    }
    return { rfNodes: rn, rfEdges: re };
  }, [hops, target]);

  async function save() {
    setErr("");
    if (hops.some((h) => h.nodes.length === 0)) {
      setErr("每跳至少一个节点");
      return;
    }
    setBusy(true);
    try {
      const payload = { name, listen_port: listen, protocol, hops, target };
      if (id) {
        await api.put(`/api/forwards/${id}`, payload);
      } else {
        await api.post("/api/forwards", payload);
      }
      navigate("/forwards");
    } catch (e: any) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6">
      <header className="flex justify-between items-end">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">
            {id ? (readOnly ? "Forward Detail" : "Edit Forward") : "New Forward"}
          </div>
          <h1 className="text-2xl font-semibold mt-1">
            {id ? `转发 #${id}` : "新建转发"}
          </h1>
        </div>
        {!readOnly && (
          <button className="btn-primary" disabled={busy} onClick={save}>
            {busy ? "保存中…" : id ? "保存修改" : "保存转发"}
          </button>
        )}
      </header>

      <div className="card grid grid-cols-1 md:grid-cols-4 gap-4">
        <div>
          <label className="label">名称</label>
          <input
            className="input"
            disabled={readOnly}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div>
          <label className="label">监听端口</label>
          <input
            type="number"
            className="input"
            disabled={readOnly}
            value={listen}
            onChange={(e) => setListen(parseInt(e.target.value) || 0)}
          />
        </div>
        <div>
          <label className="label">协议</label>
          <select
            className="input"
            disabled={readOnly}
            value={protocol}
            onChange={(e) => setProtocol(e.target.value)}
          >
            <option value="tcp">TCP</option>
            <option value="udp">UDP</option>
          </select>
        </div>
        <div>
          <label className="label">目标 host:port</label>
          <input
            className="input"
            disabled={readOnly}
            placeholder="1.2.3.4:22"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          />
        </div>
      </div>

      <div className="space-y-3">
        <div className="flex justify-between items-center">
          <h2 className="text-xs uppercase tracking-[0.18em] text-mute font-mono">
            跳路径 (Hops)
          </h2>
          {!readOnly && (
            <button className="btn-secondary" onClick={addHop}>
              + 添加一跳
            </button>
          )}
        </div>
        {hops.map((h, hi) => (
          <div key={hi} className="card space-y-3">
            <div className="flex justify-between items-center">
              <div className="text-sm font-medium">
                第 <span className="text-accent">{hi + 1}</span> 跳
              </div>
              <div className="flex gap-2 items-center">
                <select
                  className="input !py-1.5 !w-auto !text-xs"
                  disabled={readOnly}
                  value={h.strategy}
                  onChange={(e) => setHopStrategy(hi, e.target.value)}
                >
                  {STRATEGIES.map((s) => (
                    <option key={s.v} value={s.v}>
                      {s.label}
                    </option>
                  ))}
                </select>
                {!readOnly && hops.length > 1 && (
                  <button className="btn-danger !py-1.5 !px-3 text-xs" onClick={() => rmHop(hi)}>
                    删除跳
                  </button>
                )}
              </div>
            </div>
            <div className="flex flex-wrap gap-2 min-h-[40px]">
              {h.nodes.map((n, ni) => (
                <div
                  key={n.id}
                  className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-panel2 border border-line"
                >
                  <span className="font-mono text-sm">{n.id}</span>
                  <span className="text-mute text-xs">权重</span>
                  <input
                    type="number"
                    min="1"
                    disabled={readOnly}
                    value={n.weight}
                    onChange={(e) => setNodeWeight(hi, ni, parseInt(e.target.value) || 1)}
                    className="w-14 bg-bg border border-line rounded px-1 py-0.5 text-xs text-fg"
                  />
                  {!readOnly && (
                    <button
                      className="text-mute hover:text-danger text-sm"
                      onClick={() => rmNode(hi, ni)}
                    >
                      ✕
                    </button>
                  )}
                </div>
              ))}
              {!readOnly && (
                <select
                  className="input !py-1.5 !w-auto !text-xs"
                  value=""
                  onChange={(e) => addNode(hi, e.target.value)}
                >
                  <option value="">+ 添加节点…</option>
                  {nodes
                    .filter((n) => !h.nodes.some((x) => x.id === n.id))
                    .map((n) => (
                      <option key={n.id} value={n.id}>
                        {n.id} ({n.name})
                      </option>
                    ))}
                </select>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="card p-0 h-[360px] overflow-hidden">
        <ReactFlow
          nodes={rfNodes}
          edges={rfEdges}
          fitView
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background color="#222933" gap={20} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>

      {err && <div className="text-danger text-sm">{err}</div>}
    </div>
  );
}
