import { useEffect, useState } from "react";
import { api, type User } from "../lib/api";

export default function Users() {
  const [list, setList] = useState<User[] | null>(null);
  useEffect(() => {
    api.get<User[]>("/api/users").then(setList).catch(() => setList([]));
  }, []);

  return (
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-8 animate-slide-up">
      <header>
        <p className="eyebrow">access</p>
        <h1 className="text-2xl tracking-tight font-medium mt-1">Users</h1>
      </header>

      {list === null ? (
        <Skel />
      ) : list.length === 0 ? (
        <div className="border border-dashed border-line rounded-md px-6 py-16 text-center">
          <p className="text-sm text-ink-1 font-medium">No users yet</p>
          <p className="text-xs text-ink-3 mt-1">通过邀请码邀请客户加入</p>
        </div>
      ) : (
        <div className="border-t border-line">
          <div className="grid grid-cols-[0.5fr_1.5fr_0.8fr_1.4fr] gap-x-4 py-2 px-2 table-h border-b border-line">
            <span>ID</span>
            <span>Username</span>
            <span>Role</span>
            <span>Created</span>
          </div>
          <ul className="divide-y divide-line">
            {list.map((u) => (
              <li key={u.id} className="row-hover">
                <div className="grid grid-cols-[0.5fr_1.5fr_0.8fr_1.4fr] gap-x-4 items-center py-3 px-2">
                  <span className="num text-sm text-ink-2">#{u.id}</span>
                  <span className="text-sm font-medium">{u.username}</span>
                  <span className={u.role === "admin" ? "tag-warn" : "tag-ok"}>{u.role}</span>
                  <span className="num text-xs text-ink-3">
                    {u.created_at ? new Date(u.created_at).toLocaleString() : "—"}
                  </span>
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
          <div className="skel h-4 w-1/3 mb-2" />
          <div className="skel h-3 w-1/2" />
        </li>
      ))}
    </ul>
  );
}
