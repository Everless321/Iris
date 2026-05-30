import { useEffect, useState } from "react";
import { api, type User } from "../lib/api";

export default function Users() {
  const [list, setList] = useState<User[]>([]);
  useEffect(() => {
    api.get<User[]>("/api/users").then(setList).catch(() => {});
  }, []);

  return (
    <div className="space-y-6">
      <header>
        <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Users</div>
        <h1 className="text-2xl font-semibold mt-1">用户列表</h1>
      </header>
      <div className="card overflow-x-auto p-0">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wider text-mute font-mono bg-panel2">
            <tr>
              <th className="text-left px-4 py-3">ID</th>
              <th className="text-left px-4 py-3">用户名</th>
              <th className="text-left px-4 py-3">角色</th>
              <th className="text-left px-4 py-3">创建时间</th>
            </tr>
          </thead>
          <tbody>
            {list.map((u) => (
              <tr key={u.id} className="table-row">
                <td className="px-4 py-3 font-mono">{u.id}</td>
                <td className="px-4 py-3 font-medium">{u.username}</td>
                <td className="px-4 py-3">
                  <span className={u.role === "admin" ? "pill-warn" : "pill-ok"}>{u.role}</span>
                </td>
                <td className="px-4 py-3 font-mono text-xs text-dim">
                  {u.created_at ? new Date(u.created_at).toLocaleString() : "—"}
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={4} className="text-mute text-center py-8">
                  暂无用户
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
