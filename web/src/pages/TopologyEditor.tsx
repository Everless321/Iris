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
import { ArrowLeft, ArrowRight, Plus, Trash, X, ArrowsHorizontal } from "@phosphor-icons/react";
import { api, type Forward, type Hop, type Node } from "../lib/api";
import { useAuth } from "../lib/auth";

const PATH_STRATEGIES = [
  { v: "weighted", label: "Weighted RR" },
  { v: "source_hash", label: "Source hash" },
  { v: "least_conn", label: "Least connections" },
  { v: "latency", label: "Latency" },
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
  const [hops, setHops] = useState<Hop[]>([{ strategy: "weighted", nodes: [] }]);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.get<Node[]>("/api/nodes").catch(() => []).then((ns) => setNodes(ns as Node[]));
    if (id) {
      api.get<Forward[]>("/api/forwards").then((all) => {
        const f = all.find((x) => String(x.id) === id);
        if (f) {
          setName(f.name); setListen(f.listen_port); setProtocol(f.protocol);
          setTarget(f.target); setHops(f.hops);
        }
      });
    }
  }, [id]);

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
          ? { ...h, nodes: [...h.nodes, { id: nid, weight: 1 }] } : h
      )
    );
  };
  const setPathWeight = (pi: number, ni: number, w: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1
          ? { ...h, nodes: h.nodes.map((n, y) => (y === ni ? { ...n, weight: w } : n)) } : h
      )
    );
  const rmPathNode = (pi: number, ni: number) =>
    setHops((hs) =>
      hs.map((h, x) =>
        x === pi + 1 ? { ...h, nodes: h.nodes.filter((_, y) => y !== ni) } : h
      )
    );

  // ReactFlow viz
  const { rfNodes, rfEdges } = useMemo(() => {
    const rn: RFNode[] = [];
    const re: RFEdge[] = [];
    hops.forEach((h, hi) => {
      const isEntry = hi === 0;
      const isExit = hi === hops.length - 1;
      const label = isEntry ? "Entry (empty)" : isExit ? "Exit (empty)" : `Hop ${hi} (empty)`;
      if (h.nodes.length === 0) {
        rn.push({
          id: `h${hi}-empty`,
          data: { label },
          position: { x: hi * 220, y: 80 },
          style: {
            background: "#18181b", border: "1px dashed #3f3f46",
            color: "#71717a", borderRadius: 6, fontSize: 11, padding: 8,
            fontFamily: "Geist Mono, monospace",
          },
        });
      }
      h.nodes.forEach((n, ni) => {
        rn.push({
          id: `h${hi}-${n.id}`,
          data: {
            label: (
              <div style={{ fontSize: 11, fontFamily: "Geist Mono, monospace" }}>
                <div style={{ fontWeight: 600 }}>{n.id}</div>
                <div style={{ color: "#71717a", fontSize: 10 }}>
                  {isEntry ? "entry" : `w=${n.weight}`}
                </div>
              </div>
            ) as any,
          },
          position: { x: hi * 220, y: ni * 70 + 20 },
          style: {
            background: "#09090b",
            border: `1px solid ${isEntry ? "#06b6d4" : "#3f3f46"}`,
            color: "#fafafa", borderRadius: 6, padding: 6, width: 130,
          },
        });
      });
    });
    for (let i = 0; i < hops.length - 1; i++) {
      const left = hops[i].nodes.length ? hops[i].nodes.map((n) => `h${i}-${n.id}`) : [`h${i}-empty`];
      const right = hops[i + 1].nodes.length
        ? hops[i + 1].nodes.map((n) => `h${i + 1}-${n.id}`) : [`h${i + 1}-empty`];
      for (const a of left)
        for (const b of right)
          re.push({ id: `${a}-${b}`, source: a, target: b, style: { stroke: "#3f3f46" } });
    }
    if (target && hops.length > 0) {
      const last = hops[hops.length - 1];
      rn.push({
        id: "target",
        data: { label: `→ ${target}` },
        position: { x: hops.length * 220, y: 80 },
        style: {
          background: "#083344", border: "1px solid #06b6d4",
          color: "#cffafe", borderRadius: 6, fontSize: 11, padding: 8,
          fontFamily: "Geist Mono, monospace",
        },
      });
      const sources = last.nodes.length
        ? last.nodes.map((n) => `h${hops.length - 1}-${n.id}`)
        : [`h${hops.length - 1}-empty`];
      for (const s of sources)
        re.push({ id: `${s}-target`, source: s, target: "target", style: { stroke: "#06b6d4" } });
    }
    return { rfNodes: rn, rfEdges: re };
  }, [hops, target]);

  async function save() {
    setErr("");
    if (hops.some((h) => h.nodes.length === 0)) {
      setErr("Each hop needs at least one node");
      return;
    }
    setBusy(true);
    try {
      const payload = { name, listen_port: listen, protocol, hops, target };
      if (id) await api.put(`/api/forwards/${id}`, payload);
      else await api.post("/api/forwards", payload);
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
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-10 animate-slide-up">
      <header className="flex items-end justify-between">
        <div>
          <button
            className="btn-link text-xs mb-2"
            onClick={() => navigate("/forwards")}
          >
            <ArrowLeft size={12} /> Back
          </button>
          <p className="eyebrow">forward</p>
          <h1 className="text-2xl tracking-tight font-medium mt-1">
            {id ? `#${id}` : "New forward"}
          </h1>
        </div>
        {!readOnly && (
          <button className="btn-primary" disabled={busy} onClick={save}>
            <span>{busy ? "Saving…" : id ? "Save changes" : "Create forward"}</span>
            <ArrowRight size={14} />
          </button>
        )}
      </header>

      {/* Basic fields — no card, use spacing */}
      <section className="grid grid-cols-1 md:grid-cols-4 gap-8">
        <div>
          <label className="label">Name</label>
          <input className="field-box" disabled={readOnly} value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div>
          <label className="label">Listen port</label>
          <input
            type="number" className="field-box num" disabled={readOnly}
            value={listen} onChange={(e) => setListen(parseInt(e.target.value) || 0)}
          />
          <p className="text-[10px] text-ink-3 mt-1.5">所有入口节点都监听此端口</p>
        </div>
        <div>
          <label className="label">Protocol</label>
          <select className="field-box" disabled={readOnly} value={protocol} onChange={(e) => setProtocol(e.target.value)}>
            <option value="tcp">TCP</option>
            <option value="udp">UDP</option>
          </select>
        </div>
        <div>
          <label className="label">Target</label>
          <input
            className="field-box font-mono" disabled={readOnly} placeholder="1.2.3.4:22"
            value={target} onChange={(e) => setTarget(e.target.value)}
          />
        </div>
      </section>

      {/* Entry section */}
      <section className="border-t border-line pt-8">
        <header className="flex items-end justify-between mb-3">
          <div>
            <p className="eyebrow">entry · 入口</p>
            <h3 className="text-base font-medium tracking-tight mt-1">
              客户端连接的节点
            </h3>
          </div>
          <span className="tag-muted">{entry.nodes.length} entries</span>
        </header>
        <p className="text-xs text-ink-3 leading-relaxed max-w-[64ch] mb-4">
          所有入口节点都监听 <span className="font-mono text-ink-1">:{listen || "—"}</span>。
          客户端可连任意一个 IP（建议配 DNS A 记录多 IP 轮询）。入口段不需要 LB 策略——
          选择在客户端那一端发生。
        </p>
        <div className="flex flex-wrap gap-1.5 min-h-[36px]">
          {entry.nodes.map((n, ni) => (
            <span
              key={n.id}
              className="inline-flex items-center gap-2 px-2.5 py-1 rounded-md
                       border border-accent/30 bg-accent-bg/30 text-accent-fg
                       font-mono text-xs"
            >
              {n.id}
              {!readOnly && (
                <button
                  className="text-accent-fg/60 hover:text-danger transition-colors"
                  onClick={() => rmEntryNode(ni)}
                >
                  <X size={11} />
                </button>
              )}
            </span>
          ))}
          {!readOnly && (
            <NodePicker
              available={nodes.filter((n) => !entry.nodes.some((x) => x.id === n.id))}
              onPick={addEntryNode}
              placeholder="+ Add entry node"
            />
          )}
        </div>
      </section>

      {/* Path section */}
      <section className="border-t border-line pt-8 space-y-6">
        <header className="flex items-end justify-between">
          <div>
            <p className="eyebrow">path · 路径</p>
            <h3 className="text-base font-medium tracking-tight mt-1">
              中转 / 出口节点
            </h3>
          </div>
          {!readOnly && (
            <button className="btn-outline btn-sm" onClick={addPathHop}>
              <Plus size={12} />
              <span>Add hop</span>
            </button>
          )}
        </header>

        {pathHops.length === 0 ? (
          <p className="text-xs text-ink-3 italic">
            单跳模式：入口节点直接发到 <span className="font-mono">{target || "target"}</span>
          </p>
        ) : (
          <ul className="space-y-3">
            {pathHops.map((h, pi) => {
              const isLast = pi === pathHops.length - 1;
              return (
                <li key={pi} className="border-l-2 border-line pl-5 py-2 hover:border-line-strong transition-colors">
                  <div className="flex items-center justify-between mb-3">
                    <div className="flex items-center gap-2">
                      <span className="eyebrow">
                        {isLast ? "exit" : `hop ${pi + 1}`}
                      </span>
                      <span className="text-xs text-ink-3">
                        {isLast ? "→ target" : "→ next hop"}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <select
                        className="field-box !py-1 !text-xs !w-auto"
                        disabled={readOnly}
                        value={h.strategy}
                        onChange={(e) => setPathStrategy(pi, e.target.value)}
                      >
                        {PATH_STRATEGIES.map((s) => (
                          <option key={s.v} value={s.v}>{s.label}</option>
                        ))}
                      </select>
                      {!readOnly && (
                        <button className="btn-danger btn-sm" onClick={() => rmPathHop(pi)}>
                          <Trash size={12} />
                        </button>
                      )}
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {h.nodes.map((n, ni) => (
                      <span
                        key={n.id}
                        className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md
                                 border border-line bg-surface-1 text-ink-1
                                 font-mono text-xs"
                      >
                        {n.id}
                        <span className="text-ink-3 text-[10px]">w</span>
                        <input
                          type="number" min={1} disabled={readOnly}
                          value={n.weight}
                          onChange={(e) => setPathWeight(pi, ni, parseInt(e.target.value) || 1)}
                          className="w-9 bg-transparent border-b border-line text-ink-0 text-xs text-center
                                   focus:border-accent outline-none"
                        />
                        {!readOnly && (
                          <button
                            className="text-ink-3 hover:text-danger transition-colors"
                            onClick={() => rmPathNode(pi, ni)}
                          >
                            <X size={11} />
                          </button>
                        )}
                      </span>
                    ))}
                    {!readOnly && (
                      <NodePicker
                        available={nodes.filter((n) => !h.nodes.some((x) => x.id === n.id))}
                        onPick={(nid) => addPathNode(pi, nid)}
                        placeholder="+ Add node"
                      />
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {/* Visual */}
      <section className="border-t border-line pt-8">
        <header className="mb-3 flex items-center gap-2">
          <ArrowsHorizontal size={14} className="text-ink-3" />
          <span className="eyebrow">topology</span>
        </header>
        <div className="border border-line rounded-md h-[320px] overflow-hidden">
          <ReactFlow
            nodes={rfNodes} edges={rfEdges} fitView
            nodesDraggable={false} nodesConnectable={false} elementsSelectable={false}
            proOptions={{ hideAttribution: true }}
          >
            <Background color="#27272a" gap={18} />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
      </section>

      {err && (
        <p className="text-danger text-sm flex items-start gap-1.5">
          <span className="block w-1 self-stretch bg-danger rounded-full mt-0.5" />
          {err}
        </p>
      )}
    </div>
  );
}

function NodePicker({
  available, onPick, placeholder,
}: {
  available: Node[]; onPick: (id: string) => void; placeholder: string;
}) {
  return (
    <select
      className="field-box !py-1 !text-xs !w-auto text-ink-3 cursor-pointer hover:text-ink-0 transition-colors"
      value=""
      onChange={(e) => onPick(e.target.value)}
    >
      <option value="">{placeholder}</option>
      {available.map((n) => (
        <option key={n.id} value={n.id}>{n.id} · {n.name}</option>
      ))}
    </select>
  );
}
