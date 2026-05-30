import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { useAuth } from "../lib/auth";

const nav = [
  { to: "/", label: "Dashboard", end: true },
  { to: "/forwards", label: "我的转发" },
];
const adminNav = [
  { to: "/nodes", label: "节点" },
  { to: "/users", label: "用户" },
  { to: "/invites", label: "邀请码" },
  { to: "/sla", label: "SLA 看板" },
];

export default function Layout() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const links = user?.role === "admin" ? [...nav, ...adminNav] : nav;

  return (
    <div className="flex min-h-screen">
      <aside className="w-56 border-r border-line bg-panel/50 flex flex-col">
        <div className="p-5 border-b border-line">
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Project</div>
          <div className="text-xl font-semibold mt-1">
            Zhuan<span className="text-accent">fa</span>
          </div>
        </div>
        <nav className="flex-1 p-3 space-y-1">
          {links.map((l) => (
            <NavLink
              key={l.to}
              to={l.to}
              end={(l as any).end}
              className={({ isActive }) =>
                `block px-3 py-2 rounded-md text-sm transition ${
                  isActive ? "bg-panel2 text-fg" : "text-dim hover:text-fg hover:bg-panel2/50"
                }`
              }
            >
              {l.label}
            </NavLink>
          ))}
        </nav>
        <div className="p-3 border-t border-line text-xs">
          <div className="text-mute font-mono uppercase tracking-wider">登录</div>
          <div className="mt-1 text-fg">
            {user?.username}
            <span className="ml-2 pill-ok">{user?.role}</span>
          </div>
          <button
            className="mt-3 w-full btn-secondary"
            onClick={() => {
              logout();
              navigate("/login");
            }}
          >
            退出
          </button>
        </div>
      </aside>
      <main className="flex-1 p-8 overflow-auto">
        <Outlet />
      </main>
    </div>
  );
}
