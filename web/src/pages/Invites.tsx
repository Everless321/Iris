import { useEffect, useState } from "react";
import { Plus, Copy, Check } from "@phosphor-icons/react";
import { api, type Invite } from "../lib/api";

export default function Invites() {
  const [list, setList] = useState<Invite[] | null>(null);
  const [copied, setCopied] = useState("");
  const load = () => api.get<Invite[]>("/api/invites").then(setList).catch(() => setList([]));
  useEffect(() => { load(); }, []);

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
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-8 animate-slide-up">
      <header className="flex items-end justify-between">
        <div>
          <p className="eyebrow">access</p>
          <h1 className="text-2xl tracking-tight font-medium mt-1">Invites</h1>
        </div>
        <button className="btn-primary" onClick={gen}>
          <Plus size={14} />
          <span>New invite</span>
        </button>
      </header>

      {list === null ? (
        <Skel />
      ) : list.length === 0 ? (
        <div className="border border-dashed border-line rounded-md px-6 py-16 text-center">
          <p className="text-sm text-ink-1 font-medium">No invites yet</p>
          <p className="text-xs text-ink-3 mt-1">点击 New invite 生成第一个邀请码</p>
        </div>
      ) : (
        <div className="border-t border-line">
          <div className="grid grid-cols-[2.4fr_0.6fr_0.5fr_1.2fr_auto] gap-x-4 py-2 px-2 table-h border-b border-line">
            <span>Code</span>
            <span>Status</span>
            <span>Used by</span>
            <span>Created</span>
            <span />
          </div>
          <ul className="divide-y divide-line">
            {list.map((i) => (
              <li key={i.code} className="row-hover group">
                <div className="grid grid-cols-[2.4fr_0.6fr_0.5fr_1.2fr_auto] gap-x-4 items-center py-3 px-2">
                  <span className="num text-xs text-ink-1 truncate">{i.code}</span>
                  <span className={i.used_by ? "tag-muted" : "tag-ok"}>
                    {i.used_by ? "used" : "available"}
                  </span>
                  <span className="num text-xs text-ink-2">{i.used_by ?? "—"}</span>
                  <span className="num text-xs text-ink-3">
                    {new Date(i.created_at).toLocaleString()}
                  </span>
                  <div className="flex items-center gap-1">
                    {!i.used_by && (
                      <button
                        className="btn-outline btn-sm opacity-0 group-hover:opacity-100 transition-opacity"
                        onClick={() => copy(i.code)}
                      >
                        {copied === i.code ? <Check size={12} /> : <Copy size={12} />}
                        <span>{copied === i.code ? "Copied" : "Copy"}</span>
                      </button>
                    )}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function Skel() {
  return (
    <ul className="border-t border-line divide-y divide-line">
      {Array.from({ length: 3 }).map((_, i) => (
        <li key={i} className="py-4 px-2">
          <div className="skel h-4 w-2/3 mb-2" />
          <div className="skel h-3 w-1/4" />
        </li>
      ))}
    </ul>
  );
}
