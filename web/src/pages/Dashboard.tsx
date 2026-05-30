import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowUpRight, Plus } from "@phosphor-icons/react";
import { api, type Sla, type Forward } from "../lib/api";
import { useAuth } from "../lib/auth";

function StatusDot({ h }: { h: string }) {
  if (h === "healthy") return <span className="dot-ok" />;
  if (h === "unhealthy") return <span className="dot-bad" />;
  return <span className="dot-warn" />;
}

function HopsLine({ f }: { f: Forward }) {
  return (
    <span className="num text-[12px] text-ink-2">
      {f.hops
        .map((h, hi) => {
          const isEntry = hi === 0;
          const ids = h.nodes.map((n) => n.id).join(",");
          const txt = h.nodes.length === 1 ? h.nodes[0].id : `[${ids}]`;
          return isEntry ? txt : ` → ${txt}`;
        })
        .join("")}
    </span>
  );
}

export default function Dashboard() {
  const { user } = useAuth();
  const [sla, setSla] = useState<Sla | null>(null);
  const [fws, setFws] = useState<Forward[] | null>(null);

  useEffect(() => {
    api.get<Forward[]>("/api/forwards").then(setFws).catch(() => setFws([]));
    if (user?.role === "admin") api.get<Sla>("/api/sla").then(setSla).catch(() => {});
  }, [user]);

  const totalFails = sla?.nodes.reduce((s, n) => s + n.fail_events, 0) ?? 0;
  const avgUptime =
    sla && sla.nodes.length
      ? (sla.nodes.reduce((s, n) => s + n.uptime, 0) / sla.nodes.length) * 100
      : null;

  return (
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-12 animate-slide-up">
      {/* ── Hero ── */}
      <section>
        <p className="eyebrow mb-2">overview</p>
        <h1 className="text-3xl tracking-tighter font-medium">
          Welcome back, <span className="text-ink-2">{user?.username}</span>.
        </h1>
      </section>

      {/* ── Metrics row（divide-x，无 cards，monospace 数字）── */}
      <section className="border-t border-line">
        <div className="grid grid-cols-2 md:grid-cols-4 divide-x divide-line">
          <Metric
            label="Forwards"
            value={fws?.length}
            href="/forwards"
            footer="active"
          />
          {user?.role === "admin" && (
            <>
              <Metric
                label="Nodes"
                value={sla ? `${sla.online}/${sla.total}` : undefined}
                href="/nodes"
                footer="online"
              />
              <Metric
                label="Uptime"
                value={avgUptime != null ? `${avgUptime.toFixed(1)}%` : undefined}
                href="/sla"
                footer="avg 24h"
              />
              <Metric
                label="Fail events"
                value={totalFails}
                href="/sla"
                footer="all time"
              />
            </>
          )}
        </div>
      </section>

      {/* ── Recent forwards（divide-y rows）── */}
      <section>
        <header className="flex items-end justify-between mb-4">
          <div>
            <p className="eyebrow">forwards</p>
            <h2 className="text-base tracking-tight font-medium mt-1">Recent</h2>
          </div>
          <Link to="/forwards/new" className="btn-outline btn-sm">
            <Plus size={12} />
            <span>New</span>
          </Link>
        </header>

        {fws === null ? (
          <ul className="divide-y divide-line border-y border-line">
            {[0, 1, 2].map((i) => (
              <li key={i} className="py-3">
                <div className="skel h-4 w-1/3 mb-2" />
                <div className="skel h-3 w-2/3" />
              </li>
            ))}
          </ul>
        ) : fws.length === 0 ? (
          <EmptyState
            title="还没有转发"
            hint="创建第一条转发，把客户端流量从入口拨到目标"
            action={
              <Link to="/forwards/new" className="btn-link">
                创建转发 <ArrowUpRight size={12} />
              </Link>
            }
          />
        ) : (
          <ul className="divide-y divide-line border-y border-line">
            {fws.slice(0, 6).map((f) => (
              <li key={f.id} className="row-hover">
                <Link
                  to={`/forwards/${f.id}/edit`}
                  className="flex items-center justify-between px-2 py-3 group"
                >
                  <div className="flex items-baseline gap-3 min-w-0">
                    <span className="text-sm font-medium truncate">{f.name}</span>
                    <span className="num text-xs text-ink-3">:{f.listen_port}</span>
                  </div>
                  <div className="hidden md:flex items-center gap-4 min-w-0">
                    <HopsLine f={f} />
                    <span className="text-ink-3 num text-xs whitespace-nowrap">
                      → {f.target}
                    </span>
                    <ArrowUpRight
                      size={14}
                      className="text-ink-3 group-hover:text-ink-0 transition-colors"
                    />
                  </div>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function Metric({
  label,
  value,
  href,
  footer,
}: {
  label: string;
  value: number | string | undefined;
  href: string;
  footer: string;
}) {
  return (
    <Link
      to={href}
      className="block px-6 py-5 first:pl-0 last:pr-0 hover:bg-surface-1/40 transition-colors group"
    >
      <div className="eyebrow flex items-center justify-between">
        <span>{label}</span>
        <ArrowUpRight
          size={11}
          className="opacity-0 group-hover:opacity-100 transition-opacity"
        />
      </div>
      <div className="num text-3xl font-medium tracking-tighter mt-1.5 min-h-[2.25rem]">
        {value === undefined ? <span className="skel inline-block h-7 w-12 align-middle" /> : value}
      </div>
      <div className="eyebrow mt-1 normal-case tracking-normal text-ink-3">{footer}</div>
    </Link>
  );
}

function EmptyState({
  title,
  hint,
  action,
}: {
  title: string;
  hint: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="border border-dashed border-line rounded-md px-6 py-12 text-center">
      <p className="text-sm text-ink-1 font-medium">{title}</p>
      <p className="text-xs text-ink-3 mt-1">{hint}</p>
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
