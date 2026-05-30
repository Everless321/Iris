import { useEffect, useState } from "react";
import { api, type Invite } from "../lib/api";

export default function Invites() {
  const [list, setList] = useState<Invite[]>([]);
  const [copied, setCopied] = useState("");
  const load = () => api.get<Invite[]>("/api/invites").then(setList).catch(() => {});
  useEffect(() => {
    load();
  }, []);

  async function gen() {
    await api.post("/api/invites");
    load();
  }

  function copy(code: string) {
    navigator.clipboard.writeText(code);
    setCopied(code);
    setTimeout(() => setCopied(""), 1500);
  }

  return (
    <div className="space-y-6">
      <header className="flex justify-between items-end">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Invites</div>
          <h1 className="text-2xl font-semibold mt-1">邀请码</h1>
        </div>
        <button className="btn-primary" onClick={gen}>
          + 生成邀请码
        </button>
      </header>

      <div className="card overflow-x-auto p-0">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wider text-mute font-mono bg-panel2">
            <tr>
              <th className="text-left px-4 py-3">邀请码</th>
              <th className="text-left px-4 py-3">状态</th>
              <th className="text-left px-4 py-3">使用者 ID</th>
              <th className="text-left px-4 py-3">创建时间</th>
              <th className="px-4 py-3"></th>
            </tr>
          </thead>
          <tbody>
            {list.map((i) => (
              <tr key={i.code} className="table-row">
                <td className="px-4 py-3 font-mono text-xs">{i.code}</td>
                <td className="px-4 py-3">
                  {i.used_by ? <span className="pill-bad">已用</span> : <span className="pill-ok">未用</span>}
                </td>
                <td className="px-4 py-3 font-mono">{i.used_by ?? "—"}</td>
                <td className="px-4 py-3 text-xs text-dim font-mono">
                  {new Date(i.created_at).toLocaleString()}
                </td>
                <td className="px-4 py-3 text-right">
                  {!i.used_by && (
                    <button className="btn-secondary" onClick={() => copy(i.code)}>
                      {copied === i.code ? "已复制" : "复制"}
                    </button>
                  )}
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={5} className="text-mute text-center py-8">
                  暂无邀请码
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
