import { FormEvent, useEffect, useState } from "react";
import { api, type Node } from "../lib/api";

function HealthPill({ h }: { h: string }) {
  if (h === "healthy") return <span className="pill-ok">ok</span>;
  if (h === "unhealthy") return <span className="pill-bad">down</span>;
  return <span className="pill-warn">{h || "unknown"}</span>;
}

export default function Nodes() {
  const [list, setList] = useState<Node[]>([]);
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ id: "", name: "", addr: "", weight: 1 });
  const [err, setErr] = useState("");

  const load = () => api.get<Node[]>("/api/nodes").then(setList).catch((e) => setErr(e.message));
  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  async function onAdd(e: FormEvent) {
    e.preventDefault();
    setErr("");
    try {
      await api.post("/api/nodes", form);
      setForm({ id: "", name: "", addr: "", weight: 1 });
      setOpen(false);
      load();
    } catch (e: any) {
      setErr(e.message);
    }
  }

  async function onDel(id: string) {
    if (!confirm(`删除节点 ${id}?`)) return;
    await api.del(`/api/nodes/${id}`);
    load();
  }

  return (
    <div className="space-y-6">
      <header className="flex justify-between items-end">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Nodes</div>
          <h1 className="text-2xl font-semibold mt-1">节点管理</h1>
        </div>
        <button className="btn-primary" onClick={() => setOpen(!open)}>
          {open ? "取消" : "+ 新增节点"}
        </button>
      </header>

      {open && (
        <form onSubmit={onAdd} className="card grid grid-cols-1 md:grid-cols-5 gap-3 items-end">
          <div>
            <label className="label">ID</label>
            <input
              className="input"
              required
              value={form.id}
              onChange={(e) => setForm({ ...form, id: e.target.value })}
            />
          </div>
          <div>
            <label className="label">名称</label>
            <input
              className="input"
              required
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </div>
          <div>
            <label className="label">DataPlane 地址 (host:port)</label>
            <input
              className="input"
              required
              placeholder="1.2.3.4:7444"
              value={form.addr}
              onChange={(e) => setForm({ ...form, addr: e.target.value })}
            />
          </div>
          <div>
            <label className="label">权重</label>
            <input
              type="number"
              min="1"
              className="input"
              value={form.weight}
              onChange={(e) => setForm({ ...form, weight: parseInt(e.target.value) || 1 })}
            />
          </div>
          <button className="btn-primary">保存</button>
          {err && <div className="text-danger text-sm md:col-span-5">{err}</div>}
        </form>
      )}

      <div className="card overflow-x-auto p-0">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wider text-mute font-mono bg-panel2">
            <tr>
              <th className="text-left px-4 py-3">ID</th>
              <th className="text-left px-4 py-3">名称</th>
              <th className="text-left px-4 py-3">地址</th>
              <th className="text-left px-4 py-3">健康</th>
              <th className="text-left px-4 py-3">延迟</th>
              <th className="text-left px-4 py-3">权重</th>
              <th className="text-left px-4 py-3">可用率</th>
              <th className="px-4 py-3"></th>
            </tr>
          </thead>
          <tbody>
            {list.map((n) => (
              <tr key={n.id} className="table-row">
                <td className="px-4 py-3 font-mono">{n.id}</td>
                <td className="px-4 py-3">{n.name}</td>
                <td className="px-4 py-3 font-mono text-dim">{n.addr}</td>
                <td className="px-4 py-3">
                  <HealthPill h={n.health} />
                </td>
                <td className="px-4 py-3 font-mono">
                  {n.latency_ms != null ? `${n.latency_ms}ms` : "—"}
                </td>
                <td className="px-4 py-3">{n.weight}</td>
                <td className="px-4 py-3 font-mono text-xs">
                  {n.probe_total > 0
                    ? `${((n.probe_ok / n.probe_total) * 100).toFixed(1)}%`
                    : "—"}
                </td>
                <td className="px-4 py-3 text-right">
                  <button className="btn-danger" onClick={() => onDel(n.id)}>
                    删除
                  </button>
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={8} className="text-mute text-center py-8">
                  暂无节点
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
