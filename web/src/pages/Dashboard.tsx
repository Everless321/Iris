import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Sla, type Forward } from "../lib/api";
import { useAuth } from "../lib/auth";

export default function Dashboard() {
  const { user } = useAuth();
  const [sla, setSla] = useState<Sla | null>(null);
  const [fws, setFws] = useState<Forward[]>([]);
  useEffect(() => {
    api.get<Forward[]>("/api/forwards").then(setFws).catch(() => {});
    if (user?.role === "admin") api.get<Sla>("/api/sla").then(setSla).catch(() => {});
  }, [user]);

  return (
    <div className="space-y-8">
      <header>
        <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Dashboard</div>
        <h1 className="text-3xl font-semibold mt-1">
          欢迎，<span className="text-accent">{user?.username}</span>
        </h1>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="card">
          <div className="text-xs text-mute font-mono uppercase">我的转发</div>
          <div className="text-3xl font-semibold mt-2">{fws.length}</div>
          <Link to="/forwards" className="text-xs text-accent2 hover:underline mt-2 inline-block">
            管理 →
          </Link>
        </div>
        {user?.role === "admin" && sla && (
          <>
            <div className="card">
              <div className="text-xs text-mute font-mono uppercase">在线节点</div>
              <div className="text-3xl font-semibold mt-2">
                <span className="text-accent">{sla.online}</span>
                <span className="text-mute text-xl"> / {sla.total}</span>
              </div>
              <Link to="/nodes" className="text-xs text-accent2 hover:underline mt-2 inline-block">
                管理 →
              </Link>
            </div>
            <div className="card">
              <div className="text-xs text-mute font-mono uppercase">故障事件</div>
              <div className="text-3xl font-semibold mt-2">
                {sla.nodes.reduce((s, n) => s + n.fail_events, 0)}
              </div>
              <Link to="/sla" className="text-xs text-accent2 hover:underline mt-2 inline-block">
                SLA 看板 →
              </Link>
            </div>
          </>
        )}
      </div>

      <div>
        <h2 className="text-xs uppercase tracking-[0.18em] text-mute font-mono mb-3">
          最近转发
        </h2>
        {fws.length === 0 ? (
          <div className="card text-mute text-sm">
            还没有转发。
            <Link to="/forwards/new" className="text-accent2 hover:underline ml-1">
              创建第一条 →
            </Link>
          </div>
        ) : (
          <div className="card space-y-1">
            {fws.slice(0, 5).map((f) => (
              <div
                key={f.id}
                className="flex justify-between items-center py-2 px-1 border-b border-line last:border-0 text-sm"
              >
                <span className="font-medium">{f.name}</span>
                <span className="text-mute font-mono text-xs">
                  :{f.listen_port} → {f.hops.length} 跳 → {f.target}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
