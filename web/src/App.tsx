import { lazy, Suspense, useEffect } from "react";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";
import { useAuth } from "./lib/auth";
// StatusBoard 是公开首页，eager 加载（首屏即用）
import StatusBoard from "./pages/StatusBoard";
// 其余全部 lazy —— 游客访问 / 时不下载 admin/login 代码
const Layout = lazy(() => import("./components/Layout"));
const Login = lazy(() => import("./pages/Login"));
const Register = lazy(() => import("./pages/Register"));
const Dashboard = lazy(() => import("./pages/Dashboard"));
const Nodes = lazy(() => import("./pages/Nodes"));
const Forwards = lazy(() => import("./pages/Forwards"));
const ForwardDetail = lazy(() => import("./pages/ForwardDetail"));
const TopologyEditor = lazy(() => import("./pages/TopologyEditor"));
const Users = lazy(() => import("./pages/Users"));
const Invites = lazy(() => import("./pages/Invites"));
const SlaBoard = lazy(() => import("./pages/SlaBoard"));

const fallback = <div style={{ padding: 32, color: "#888" }}>加载中…</div>;
const lazyWrap = (el: React.ReactNode) => <Suspense fallback={fallback}>{el}</Suspense>;

function Protected({ children, admin = false }: { children: React.ReactNode; admin?: boolean }) {
  const { user, loading } = useAuth();
  const loc = useLocation();
  if (loading) return <div className="p-8 text-mute">加载中…</div>;
  if (!user) return <Navigate to="/login" state={{ from: loc }} replace />;
  if (admin && user.role !== "admin") return <Navigate to="/admin" replace />;
  return <>{children}</>;
}

/// 旧 URL 书签兼容：把 /forwards/X/Y → /admin/forwards/X/Y 等。
function LegacyRedirect({ to }: { to: string }) {
  const loc = useLocation();
  // 提取后续 path 段 (e.g. /forwards/26/edit → /26/edit)
  const prefix = "/" + loc.pathname.split("/")[1]; // /forwards
  const rest = loc.pathname.slice(prefix.length); // /26/edit
  return <Navigate to={`${to}${rest}${loc.search}`} replace />;
}

export default function App() {
  const init = useAuth((s) => s.init);
  useEffect(() => {
    init();
  }, [init]);
  return (
    <Routes>
      {/* M9 公开首页（无 auth）= 节点状态看板 */}
      <Route path="/" element={<StatusBoard />} />
      <Route path="/login" element={lazyWrap(<Login />)} />
      <Route path="/register" element={lazyWrap(<Register />)} />
      {/* 旧 URL 书签兼容：/forwards/26/edit → /admin/forwards/26/edit */}
      <Route path="/forwards/*" element={<LegacyRedirect to="/admin/forwards" />} />
      <Route path="/nodes/*" element={<LegacyRedirect to="/admin/nodes" />} />
      <Route path="/users/*" element={<LegacyRedirect to="/admin/users" />} />
      <Route path="/invites/*" element={<LegacyRedirect to="/admin/invites" />} />
      <Route path="/sla/*" element={<LegacyRedirect to="/admin/sla" />} />
      {/* 后台管理（带 auth）— 原 / 路径整体迁移到 /admin */}
      <Route
        path="/admin"
        element={
          <Protected>
            {lazyWrap(<Layout />)}
          </Protected>
        }
      >
        <Route index element={lazyWrap(<Dashboard />)} />
        <Route path="forwards" element={lazyWrap(<Forwards />)} />
        <Route path="forwards/new" element={lazyWrap(<TopologyEditor />)} />
        <Route path="forwards/:id" element={lazyWrap(<ForwardDetail />)} />
        <Route path="forwards/:id/edit" element={lazyWrap(<TopologyEditor />)} />
        <Route
          path="nodes"
          element={
            <Protected admin>
              {lazyWrap(<Nodes />)}
            </Protected>
          }
        />
        <Route
          path="users"
          element={
            <Protected admin>
              {lazyWrap(<Users />)}
            </Protected>
          }
        />
        <Route
          path="invites"
          element={
            <Protected admin>
              {lazyWrap(<Invites />)}
            </Protected>
          }
        />
        <Route
          path="sla"
          element={
            <Protected admin>
              {lazyWrap(<SlaBoard />)}
            </Protected>
          }
        />
      </Route>
    </Routes>
  );
}
