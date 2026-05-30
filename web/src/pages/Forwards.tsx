import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowUpRight, Plus, Trash } from "@phosphor-icons/react";
import { api, type Forward } from "../lib/api";

export default function Forwards() {
  const [list, setList] = useState<Forward[] | null>(null);
  const load = () => api.get<Forward[]>("/api/forwards").then(setList).catch(() => setList([]));
  useEffect(() => { load(); }, []);

  async function onDel(id: number) {
    if (!confirm(`删除转发 #${id}?`)) return;
    await api.del(`/api/forwards/${id}`);
    load();
  }

  return (
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-8 animate-slide-up">
      <header className="flex items-end justify-between">
        <div>
          <p className="eyebrow">forwards</p>
          <h1 className="text-2xl tracking-tight font-medium mt-1">Forwards</h1>
        </div>
        <Link to="/forwards/new" className="btn-primary">
          <Plus size={14} />
          <span>New forward</span>
        </Link>
      </header>

      {list === null ? (
        <Skeleton rows={4} />
      ) : list.length === 0 ? (
        <Empty />
      ) : (
        <div className="border-t border-line">
          {/* table header */}
          <div className="grid grid-cols-[1.6fr_0.6fr_0.5fr_2fr_1fr_auto] gap-x-4 py-2 px-2 table-h border-b border-line">
            <span>Name</span>
            <span>Listen</span>
            <span>Proto</span>
            <span>Path</span>
            <span>Target</span>
            <span />
          </div>
          <ul className="divide-y divide-line">
            {list.map((f) => (
              <li key={f.id} className="row-hover group">
                <div className="grid grid-cols-[1.6fr_0.6fr_0.5fr_2fr_1fr_auto] gap-x-4 items-center py-3 px-2">
                  <Link to={`/forwards/${f.id}/edit`} className="text-sm font-medium hover:text-accent-fg truncate">
                    {f.name}
                  </Link>
                  <span className="num text-xs text-ink-2">:{f.listen_port}</span>
                  <span className="tag-muted">{f.protocol}</span>
                  <PathInline f={f} />
                  <span className="num text-xs text-ink-3 truncate">{f.target}</span>
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <Link to={`/forwards/${f.id}/edit`} className="btn-ghost btn-sm" title="编辑">
                      <ArrowUpRight size={12} />
                    </Link>
                    <button className="btn-danger btn-sm" onClick={() => onDel(f.id)} title="删除">
                      <Trash size={12} />
                    </button>
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

function PathInline({ f }: { f: Forward }) {
  return (
    <div className="num text-xs flex items-center gap-1.5 flex-wrap min-w-0">
      {f.hops.map((h, hi) => {
        const ids = h.nodes.map((n) => (n.weight > 1 ? `${n.id}:${n.weight}` : n.id)).join(",");
        const txt = h.nodes.length === 1 ? h.nodes[0].id : `[${ids}]`;
        return (
          <span key={hi} className="flex items-center gap-1.5">
            {hi > 0 && <span className="text-ink-4">→</span>}
            <span className={hi === 0 ? "text-accent-fg" : "text-ink-2"}>{txt}</span>
            {hi > 0 && h.nodes.length > 1 && (
              <span className="text-ink-4 text-[10px]">@{h.strategy}</span>
            )}
          </span>
        );
      })}
    </div>
  );
}

function Skeleton({ rows }: { rows: number }) {
  return (
    <ul className="border-t border-line divide-y divide-line">
      {Array.from({ length: rows }).map((_, i) => (
        <li key={i} className="py-4 px-2">
          <div className="skel h-4 w-1/4 mb-2" />
          <div className="skel h-3 w-2/3" />
        </li>
      ))}
    </ul>
  );
}

function Empty() {
  return (
    <div className="border border-dashed border-line rounded-md px-6 py-16 text-center">
      <p className="text-sm text-ink-1 font-medium">No forwards yet</p>
      <p className="text-xs text-ink-3 mt-1 mb-4 max-w-[36ch] mx-auto">
        创建第一条转发——选入口节点、配下游路径、指定目标地址
      </p>
      <Link to="/forwards/new" className="btn-link">
        Create your first forward <ArrowUpRight size={12} />
      </Link>
    </div>
  );
}
