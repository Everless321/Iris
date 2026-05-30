import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, type Forward } from "../lib/api";

export default function Forwards() {
  const [list, setList] = useState<Forward[]>([]);
  const load = () => api.get<Forward[]>("/api/forwards").then(setList).catch(() => {});
  useEffect(() => {
    load();
  }, []);

  async function onDel(id: number) {
    if (!confirm(`删除转发 #${id}?`)) return;
    await api.del(`/api/forwards/${id}`);
    load();
  }

  return (
    <div className="space-y-6">
      <header className="flex justify-between items-end">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Forwards</div>
          <h1 className="text-2xl font-semibold mt-1">转发管理</h1>
        </div>
        <Link to="/forwards/new" className="btn-primary">
          + 新建转发
        </Link>
      </header>

      <div className="card overflow-x-auto p-0">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wider text-mute font-mono bg-panel2">
            <tr>
              <th className="text-left px-4 py-3">名称</th>
              <th className="text-left px-4 py-3">监听</th>
              <th className="text-left px-4 py-3">协议</th>
              <th className="text-left px-4 py-3">路径</th>
              <th className="text-left px-4 py-3">目标</th>
              <th className="px-4 py-3"></th>
            </tr>
          </thead>
          <tbody>
            {list.map((f) => (
              <tr key={f.id} className="table-row">
                <td className="px-4 py-3 font-medium">{f.name}</td>
                <td className="px-4 py-3 font-mono">:{f.listen_port}</td>
                <td className="px-4 py-3">
                  <span className="pill-ok">{f.protocol}</span>
                </td>
                <td className="px-4 py-3 font-mono text-xs text-dim">
                  {f.hops.map((h, hi) => {
                    const isEntry = hi === 0;
                    const txt =
                      h.nodes.length === 1
                        ? h.nodes[0].id
                        : `[${h.nodes
                            .map((n) => (n.weight > 1 ? `${n.id}:${n.weight}` : n.id))
                            .join(",")}${isEntry ? "" : `@${h.strategy}`}]`;
                    return (
                      <span key={hi}>
                        {hi > 0 && <span className="text-mute"> → </span>}
                        <span className={isEntry ? "text-accent" : ""}>
                          {isEntry ? `⏵ ${txt}` : txt}
                        </span>
                      </span>
                    );
                  })}
                </td>
                <td className="px-4 py-3 font-mono text-dim">{f.target}</td>
                <td className="px-4 py-3 text-right space-x-2">
                  <Link to={`/forwards/${f.id}/edit`} className="btn-secondary">
                    查看
                  </Link>
                  <button className="btn-danger" onClick={() => onDel(f.id)}>
                    删除
                  </button>
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={6} className="text-mute text-center py-8">
                  暂无转发，
                  <Link to="/forwards/new" className="text-accent2 hover:underline">
                    创建第一条
                  </Link>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
