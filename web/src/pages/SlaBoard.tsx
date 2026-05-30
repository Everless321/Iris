import { useEffect, useState } from "react";
import { LineChart, Line, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid } from "recharts";
import { api, type Sla } from "../lib/api";

type Point = { ts: number; latency_ms: number | null; ok: number };
type Samples = Record<string, Point[]>;

export default function SlaBoard() {
  const [sla, setSla] = useState<Sla | null>(null);
  const [samples, setSamples] = useState<Samples>({});

  useEffect(() => {
    const load = async () => {
      try {
        const s = await api.get<Sla>("/api/sla");
        setSla(s);
        const sm = await api.get<Samples>("/api/sla/samples");
        setSamples(sm);
      } catch {}
    };
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  if (!sla) return <div className="text-mute">加载中…</div>;

  return (
    <div className="space-y-6">
      <header>
        <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">SLA Board</div>
        <h1 className="text-2xl font-semibold mt-1">服务质量看板</h1>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="card">
          <div className="text-xs text-mute font-mono uppercase">在线节点</div>
          <div className="text-3xl font-semibold mt-2">
            <span className="text-accent">{sla.online}</span>
            <span className="text-mute text-xl"> / {sla.total}</span>
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-mute font-mono uppercase">总故障事件</div>
          <div className="text-3xl font-semibold mt-2">
            {sla.nodes.reduce((s, n) => s + n.fail_events, 0)}
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-mute font-mono uppercase">平均可用率</div>
          <div className="text-3xl font-semibold mt-2 text-accent">
            {sla.nodes.length > 0
              ? (
                  (sla.nodes.reduce((s, n) => s + n.uptime, 0) / sla.nodes.length) *
                  100
                ).toFixed(1) + "%"
              : "—"}
          </div>
        </div>
      </div>

      <h2 className="text-xs uppercase tracking-[0.18em] text-mute font-mono">节点延迟（近 1 小时）</h2>
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {sla.nodes.map((n) => {
          const pts = (samples[n.id] || []).map((p) => ({
            t: new Date(p.ts).toLocaleTimeString().slice(0, 5),
            latency: p.ok ? p.latency_ms : null,
          }));
          return (
            <div key={n.id} className="card">
              <div className="flex justify-between items-center mb-3">
                <div>
                  <div className="font-medium">{n.name}</div>
                  <div className="text-xs text-mute font-mono">{n.id}</div>
                </div>
                <div className="text-right text-xs">
                  <div className={n.health === "healthy" ? "pill-ok" : "pill-bad"}>{n.health}</div>
                  <div className="text-dim mt-1 font-mono">{(n.uptime * 100).toFixed(1)}%</div>
                </div>
              </div>
              <ResponsiveContainer width="100%" height={160}>
                <LineChart data={pts}>
                  <CartesianGrid stroke="#222933" strokeDasharray="3 3" />
                  <XAxis
                    dataKey="t"
                    stroke="#5b6470"
                    tick={{ fontSize: 10 }}
                    interval="preserveStartEnd"
                  />
                  <YAxis stroke="#5b6470" tick={{ fontSize: 10 }} unit="ms" />
                  <Tooltip
                    contentStyle={{ background: "#12161c", border: "1px solid #222933", fontSize: 12 }}
                    labelStyle={{ color: "#8a93a3" }}
                  />
                  <Line
                    type="monotone"
                    dataKey="latency"
                    stroke="#5ad6ff"
                    strokeWidth={2}
                    dot={false}
                    connectNulls={false}
                  />
                </LineChart>
              </ResponsiveContainer>
            </div>
          );
        })}
        {sla.nodes.length === 0 && <div className="text-mute">暂无节点</div>}
      </div>
    </div>
  );
}
