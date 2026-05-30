import { NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";
import {
  Gauge,
  ArrowsLeftRight,
  HardDrives,
  UsersThree,
  Ticket,
  ChartLine,
  SignOut,
  type Icon,
} from "@phosphor-icons/react";
import { useAuth } from "../lib/auth";

type Item = { to: string; label: string; icon: Icon; end?: boolean };

const nav: Item[] = [
  { to: "/", label: "Overview", icon: Gauge, end: true },
  { to: "/forwards", label: "Forwards", icon: ArrowsLeftRight },
];
const adminNav: Item[] = [
  { to: "/nodes", label: "Nodes", icon: HardDrives },
  { to: "/users", label: "Users", icon: UsersThree },
  { to: "/invites", label: "Invites", icon: Ticket },
  { to: "/sla", label: "SLA", icon: ChartLine },
];

const TITLES: Record<string, string> = {
  "/": "Overview",
  "/forwards": "Forwards",
  "/forwards/new": "New forward",
  "/nodes": "Nodes",
  "/users": "Users",
  "/invites": "Invites",
  "/sla": "SLA",
};

function pageTitle(path: string): string {
  if (TITLES[path]) return TITLES[path];
  if (path.startsWith("/forwards/") && path.endsWith("/edit")) return "Edit forward";
  return "";
}

export default function Layout() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const loc = useLocation();
  const links = user?.role === "admin" ? [...nav, ...adminNav] : nav;
  const title = pageTitle(loc.pathname);

  return (
    <div className="flex min-h-[100dvh]">
      {/* —— 左侧细图标栏 —— */}
      <aside className="w-14 shrink-0 border-r border-line bg-surface-0 flex flex-col items-center py-4">
        <div className="mb-6 select-none" title="zhuanfa">
          <div className="w-7 h-7 rounded-md bg-ink-0 text-surface-0 flex items-center justify-center font-mono text-xs font-bold">
            z
          </div>
        </div>
        <nav className="flex-1 flex flex-col gap-1">
          {links.map((l) => (
            <NavLink
              key={l.to}
              to={l.to}
              end={l.end}
              title={l.label}
              className={({ isActive }) =>
                `group relative w-10 h-10 rounded-md flex items-center justify-center transition-colors
                 ${isActive ? "bg-surface-2 text-ink-0" : "text-ink-3 hover:text-ink-0 hover:bg-surface-1"}`
              }
            >
              {({ isActive }) => (
                <>
                  <l.icon size={18} weight={isActive ? "fill" : "regular"} />
                  {/* hover tooltip on right */}
                  <span className="absolute left-full ml-2 px-2 py-1 bg-surface-2 border border-line rounded text-xs whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-10">
                    {l.label}
                  </span>
                </>
              )}
            </NavLink>
          ))}
        </nav>
        <button
          onClick={() => {
            logout();
            navigate("/login");
          }}
          title="Sign out"
          className="w-10 h-10 rounded-md flex items-center justify-center text-ink-3 hover:text-danger hover:bg-surface-1 transition-colors"
        >
          <SignOut size={18} />
        </button>
      </aside>

      {/* —— 主区 —— */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* 顶栏 */}
        <header className="h-14 shrink-0 border-b border-line flex items-center justify-between px-8">
          <div className="flex items-center gap-3 min-w-0">
            <span className="text-sm font-medium tracking-tight truncate">{title}</span>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <span className="text-ink-3 font-mono">{user?.username}</span>
            <span
              className={`tag ${user?.role === "admin" ? "tag-warn" : "tag-ok"}`}
            >
              {user?.role}
            </span>
          </div>
        </header>

        <main className="flex-1 overflow-auto animate-fade-in">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
