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

const PATH_STRATEGIES = [
  { v: "weighted", label: "加权轮询" },
  { v: "source_hash", label: "会话保持(源IP哈希)" },
  { v: "least_conn", label: "最小连接数" },
  { v: "latency", label: "延迟最优" },
];

export default function TopologyEditor() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { user } = useAuth();
  const readOnly = !!id && user?.role !== "admin";
  const [nodes, setNodes] = useState<Node[]>([]);
  const [name, setName] = useState("");
  const [listen, setListen] = useState(10080);
  const [protocol, setProtocol] = useState("tcp");
  const [target, setTarget] = useState("");
  // hops[0] = 入口节点组（客户端连接的地址；strategy 字段运行时不生效）
  // hops[1..] = 中转/出口节点组（受 LB 策略控制）
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

  // —— 入口节点（hops[0]）操作 ——
  function addEntryNode(nodeId: string) {
    if (!nodeId) return;
    setHops((hs) => {
      const entry = hs[0] ?? { strategy: "weighted", nodes: [] };
      if (entry.nodes.some((n) => n.id === nodeId)) return hs;
      return [{ ...entry, nodes: [...entry.nodes, { id: nodeId, weight: 1 }] }, ...hs.slice(1)];
    });
  }
  function rmEntryNode(ni: number) {
    setHops((hs) => [
      { ...hs[0], nodes: hs[0].nodes.filter((_, x) => x !== ni) },
      ...hs.slice(1),
    ]);
  }
  function setEntryWeight(ni: number, w: number) {
    setHops((hs) => [
      { ...hs[0], nodes: hs[0].nodes.map((n, x) => (x === ni ? { ...n, weight: w } : n)) },
      ...hs.slice(1),
    ]);
  }

  // —— 路径跳（hops[1..]）操作 ——
  function addPathHop() {
    setHops((hs) => [...hs, { strategy: "weighted", nodes: [] }]);
  }
  function rmPathHop(pi: number) {
    // pi 是路径序号（基于 0），实际 hop index = pi + 1
    setHops((hs) => hs.filter((_, x) => x !== pi + 1));
  }
  function setPathStrategy(pi: number, s: string) {
    setHops((hs) => hs.map((h, x) => (x === pi + 1 ? { ...h, strategy: s } : h)));
  }
  function addPathNode(pi: number, nodeId: string) {
    if (!nodeId) return;
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1 && !h.nodes.some((n) => n.id === nodeId)
          ? { ...h, nodes: [...h.nodes, { id: nodeId, weight: 1 }] }
          : h
      )
    );
  }
  function setPathWeight(pi: number, ni: number, w: number) {
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1
          ? { ...h, nodes: h.nodes.map((n, y) => (y === ni ? { ...n, weight: w } : n)) }
          : h
      )
    );
  }
  function rmPathNode(pi: number, ni: number) {
    setHops((hs) =>
      hs.map((h, x) => (x === pi + 1 ? { ...h, nodes: h.nodes.filter((_, y) => y !== ni) } : h))
    );
  }

  // —— React Flow 拓扑可视化 ——
  const { rfNodes, rfEdges } = useMemo(() => {
    const rn: RFNode[] = [];
    const re: RFEdge[] = [];
    hops.forEach((h, hi) => {
      const label = hi === 0 ? "入口 (空)" : hi === hops.length - 1 ? `出口 (空)` : `第 ${hi} 跳 (空)`;
      if (h.nodes.length === 0) {
        rn.push({
          id: `h${hi}-empty`,
          data: { label },
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
        const isEntry = hi === 0;
        rn.push({
          id,
          data: {
            label: (
              <div style={{ fontSize: 11 }}>
                <div style={{ fontWeight: 600 }}>{n.id}</div>
                <div style={{ color: "#8a93a3", fontSize: 10 }}>
                  {isEntry ? "入口" : `w=${n.weight}`}
                </div>
              </div>
            ) as any,
          },
          position: { x: hi * 220, y: ni * 70 + 20 },
          style: {
            background: "#12161c",
            border: `1px solid ${isEntry ? "#7cf3a0" : "#5ad6ff"}`,
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
      setErr("每跳至少一个节点（入口和路径都不能为空）");
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

  const entry = hops[0] ?? { strategy: "weighted", nodes: [] };
  const pathHops = hops.slice(1);

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
          <input className="input" disabled={readOnly} value={name} onChange={(e) => setName(e.target.value)} />
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
          <div className="text-[10px] text-mute mt-1">所有入口节点都会监听此端口</div>
        </div>
        <div>
          <label className="label">协议</label>
          <select className="input" disabled={readOnly} value={protocol} onChange={(e) => setProtocol(e.target.value)}>
            <option value="tcp">TCP</option>
            <option value="udp">UDP</option>
          </select>
        </div>
        <div>
          <label className="label">目标 host:port</label>
          <input className="input" disabled={readOnly} placeholder="1.2.3.4:22"
            value={target} onChange={(e) => setTarget(e.target.value)} />
        </div>
      </div>

      {/* 入口节点段 */}
      <div className="card space-y-3 border-accent/30">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Entry · 入口</div>
            <h3 className="text-base font-semibold mt-0.5">入口节点 <span className="text-mute text-xs font-normal">（客户端连接的地址）</span></h3>
          </div>
          <span className="pill-ok">{entry.nodes.length} 个入口</span>
        </div>
        <div className="text-xs text-dim bg-panel2/50 border border-line rounded-md p-3 leading-relaxed">
          所有入口节点都监听同一个端口 <span className="font-mono text-fg">:{listen || "—"}</span>。
          客户端可连接其中<b>任意一个</b>的 IP（建议配 DNS A 记录多 IP 轮询，或给客户端一份地址列表）。
          入口的"挑选"在客户端这一端发生，平台不参与，因此<b>入口段不需要 LB 策略</b>。
        </div>
        <div className="flex flex-wrap gap-2 min-h-[40px]">
          {entry.nodes.map((n, ni) => (
            <div key={n.id} className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-panel2 border border-accent/30">
              <span className="font-mono text-sm text-accent">{n.id}</span>
              {!readOnly && (
                <button className="text-mute hover:text-danger text-sm" onClick={() => rmEntryNode(ni)} title="移除">
                  ✕
                </button>
              )}
            </div>
          ))}
          {!readOnly && (
            <select className="input !py-1.5 !w-auto !text-xs" value="" onChange={(e) => addEntryNode(e.target.value)}>
              <option value="">+ 添加入口节点…</option>
              {nodes.filter((n) => !entry.nodes.some((x) => x.id === n.id)).map((n) => (
                <option key={n.id} value={n.id}>{n.id} ({n.name})</option>
              ))}
            </select>
          )}
        </div>
        {/* 隐藏：入口的 weight 在 UI 上不暴露——客户端选择，与服务端权重无关 */}
        {entry.nodes.some((n) => n.weight !== 1) && (
          <details className="text-xs text-mute">
            <summary className="cursor-pointer hover:text-fg">高级：调整入口权重 (运行时不生效，仅用于文档化)</summary>
            <div className="flex gap-2 mt-2 flex-wrap">
              {entry.nodes.map((n, ni) => (
                <label key={n.id} className="flex items-center gap-1 text-xs">
                  <span className="font-mono">{n.id}</span>
                  <input type="number" min="1" disabled={readOnly} className="w-12 bg-bg border border-line rounded px-1 py-0.5 text-fg"
                    value={n.weight} onChange={(e) => setEntryWeight(ni, parseInt(e.target.value) || 1)} />
                </label>
              ))}
            </div>
          </details>
        )}
      </div>

      {/* 路径段 */}
      <div className="space-y-3">
        <div className="flex justify-between items-center">
          <div>
            <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Path · 路径</div>
            <h3 className="text-base font-semibold mt-0.5">中转 / 出口节点 <span className="text-mute text-xs font-normal">（流量进入入口后，依次穿过）</span></h3>
          </div>
          {!readOnly && (
            <button className="btn-secondary" onClick={addPathHop}>+ 添加下一跳</button>
          )}
        </div>
        {pathHops.length === 0 && (
          <div className="text-xs text-mute italic p-3">
            还没有路径——目前是单跳模式，入口节点会直接把流量发到目标 <span className="font-mono">{target || "host:port"}</span>。
          </div>
        )}
        {pathHops.map((h, pi) => {
          const isLast = pi === pathHops.length - 1;
          return (
            <div key={pi} className="card space-y-3">
              <div className="flex justify-between items-center">
                <div className="text-sm font-medium">
                  {isLast ? <>出口跳 <span className="text-accent">（流量在这里出去到目标）</span></> : <>第 <span className="text-accent">{pi + 1}</span> 跳（中转）</>}
                </div>
                <div className="flex gap-2 items-center">
                  <select
                    className="input !py-1.5 !w-auto !text-xs"
                    disabled={readOnly}
                    value={h.strategy}
                    onChange={(e) => setPathStrategy(pi, e.target.value)}
                  >
                    {PATH_STRATEGIES.map((s) => (
                      <option key={s.v} value={s.v}>{s.label}</option>
                    ))}
                  </select>
                  {!readOnly && (
                    <button className="btn-danger !py-1.5 !px-3 text-xs" onClick={() => rmPathHop(pi)}>
                      删除此跳
                    </button>
                  )}
                </div>
              </div>
              <div className="flex flex-wrap gap-2 min-h-[40px]">
                {h.nodes.map((n, ni) => (
                  <div key={n.id} className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-panel2 border border-line">
                    <span className="font-mono text-sm">{n.id}</span>
                    <span className="text-mute text-xs">w</span>
                    <input type="number" min="1" disabled={readOnly}
                      value={n.weight}
                      onChange={(e) => setPathWeight(pi, ni, parseInt(e.target.value) || 1)}
                      className="w-14 bg-bg border border-line rounded px-1 py-0.5 text-xs text-fg" />
                    {!readOnly && (
                      <button className="text-mute hover:text-danger text-sm" onClick={() => rmPathNode(pi, ni)}>✕</button>
                    )}
                  </div>
                ))}
                {!readOnly && (
                  <select className="input !py-1.5 !w-auto !text-xs" value="" onChange={(e) => addPathNode(pi, e.target.value)}>
                    <option value="">+ 添加节点…</option>
                    {nodes.filter((n) => !h.nodes.some((x) => x.id === n.id)).map((n) => (
                      <option key={n.id} value={n.id}>{n.id} ({n.name})</option>
                    ))}
                  </select>
                )}
              </div>
            </div>
          );
        })}
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
