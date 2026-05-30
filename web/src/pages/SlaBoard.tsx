import { useEffect, useState } from "react";
import { LineChart, Line, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid } from "recharts";
import { api, type Sla } from "../lib/api";

type Point = { ts: number; latency_ms: number | null; ok: number };
type Samples = Record<string, Point[]>;

function StatusDot({ h }: { h: string }) {
  if (h === "healthy") return <span className="dot-ok" />;
  if (h === "unhealthy") return <span className="dot-bad" />;
  return <span className="dot-unknown" />;
}

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

  if (!sla) return <Skel />;

  const totalFails = sla.nodes.reduce((s, n) => s + n.fail_events, 0);
  const avgUptime =
    sla.nodes.length > 0
      ? (sla.nodes.reduce((s, n) => s + n.uptime, 0) / sla.nodes.length) * 100
      : null;

  return (
    <div className="px-8 py-10 max-w-[1400px] mx-auto space-y-12 animate-slide-up">
      <header>
        <p className="eyebrow">monitoring</p>
        <h1 className="text-2xl tracking-tight font-medium mt-1">SLA</h1>
      </header>

      {/* Metrics */}
      <section className="border-t border-line">
        <div className="grid grid-cols-3 divide-x divide-line">
          <Stat label="Nodes online" value={`${sla.online}/${sla.total}`} footer="healthy" />
          <Stat
            label="Average uptime"
            value={avgUptime != null ? `${avgUptime.toFixed(2)}%` : "—"}
            footer="all time"
          />
          <Stat label="Fail events" value={totalFails} footer="all time" />
        </div>
      </section>

      {/* Per-node charts */}
      <section className="space-y-4">
        <header>
          <p className="eyebrow">latency</p>
          <h2 className="text-base tracking-tight font-medium mt-1">Per node · last 1h</h2>
        </header>

        {sla.nodes.length === 0 ? (
          <div className="border border-dashed border-line rounded-md px-6 py-16 text-center text-xs text-ink-3">
            还没有节点
          </div>
        ) : (
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            {sla.nodes.map((n) => {
              const pts = (samples[n.id] || []).map((p) => ({
                t: new Date(p.ts).toLocaleTimeString().slice(0, 5),
                latency: p.ok ? p.latency_ms : null,
              }));
              return (
                <div
                  key={n.id}
                  className="border border-line rounded-md p-5 hover:border-line-strong transition-colors"
                >
                  <div className="flex items-start justify-between mb-4">
                    <div>
                      <div className="text-sm font-medium tracking-tight flex items-center gap-2">
                        <StatusDot h={n.health} /> {n.name}
                      </div>
                      <div className="num text-xs text-ink-3 mt-0.5">{n.id}</div>
                    </div>
                    <div className="text-right">
                      <div className="num text-lg tracking-tighter">
                        {(n.uptime * 100).toFixed(1)}
                        <span className="text-ink-3 text-xs">%</span>
                      </div>
                      <div className="eyebrow normal-case tracking-normal text-ink-3 mt-0.5">uptime</div>
                    </div>
                  </div>

                  <ResponsiveContainer width="100%" height={140}>
                    <LineChart data={pts} margin={{ top: 4, right: 4, left: -16, bottom: 0 }}>
                      <CartesianGrid stroke="#27272a" strokeDasharray="2 3" vertical={false} />
                      <XAxis
                        dataKey="t"
                        stroke="#52525b"
                        tick={{ fontSize: 10, fontFamily: "Geist Mono, monospace" }}
                        tickLine={false}
                        axisLine={false}
                        interval="preserveStartEnd"
                      />
                      <YAxis
                        stroke="#52525b"
                        tick={{ fontSize: 10, fontFamily: "Geist Mono, monospace" }}
                        tickLine={false}
                        axisLine={false}
                        unit="ms"
                        width={40}
                      />
                      <Tooltip
                        contentStyle={{
                          background: "#18181b",
                          border: "1px solid #27272a",
                          fontSize: 11,
                          borderRadius: 6,
                        }}
                        labelStyle={{ color: "#a1a1aa" }}
                        cursor={{ stroke: "#3f3f46" }}
                      />
                      <Line
                        type="monotone"
                        dataKey="latency"
                        stroke="#06b6d4"
                        strokeWidth={1.5}
                        dot={false}
                        connectNulls={false}
                        isAnimationActive={false}
                      />
                    </LineChart>
                  </ResponsiveContainer>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}

function Stat({ label, value, footer }: { label: string; value: string | number; footer: string }) {
  return (
    <div className="px-6 py-5 first:pl-0 last:pr-0">
      <div className="eyebrow">{label}</div>
      <div className="num text-3xl font-medium tracking-tighter mt-1.5">{value}</div>
      <div className="eyebrow normal-case tracking-normal text-ink-3 mt-1">{footer}</div>
    </div>
  );
}

function Skel() {
  return (
    <div className="px-8 py-10 space-y-12">
      <div className="skel h-8 w-32" />
      <div className="grid grid-cols-3 gap-6">
        {[0, 1, 2].map((i) => (
          <div key={i} className="skel h-24" />
        ))}
      </div>
    </div>
  );
}
