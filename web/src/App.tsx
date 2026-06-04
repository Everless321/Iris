import { useEffect } from "react";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";
import { useAuth } from "./lib/auth";
import Layout from "./components/Layout";
import Login from "./pages/Login";
import Register from "./pages/Register";
import Dashboard from "./pages/Dashboard";
import Nodes from "./pages/Nodes";
import Forwards from "./pages/Forwards";
import ForwardDetail from "./pages/ForwardDetail";
import TopologyEditor from "./pages/TopologyEditor";
import Users from "./pages/Users";
import Invites from "./pages/Invites";
import SlaBoard from "./pages/SlaBoard";
import StatusBoard from "./pages/StatusBoard";

function Protected({ children, admin = false }: { children: React.ReactNode; admin?: boolean }) {
  const { user, loading } = useAuth();
  const loc = useLocation();
  if (loading) return <div className="p-8 text-mute">加载中…</div>;
  if (!user) return <Navigate to="/login" state={{ from: loc }} replace />;
  if (admin && user.role !== "admin") return <Navigate to="/admin" replace />;
  return <>{children}</>;
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
      <Route path="/login" element={<Login />} />
      <Route path="/register" element={<Register />} />
      {/* 后台管理（带 auth）— 原 / 路径整体迁移到 /admin */}
      <Route
        path="/admin"
        element={
          <Protected>
            <Layout />
          </Protected>
        }
      >
        <Route index element={<Dashboard />} />
        <Route path="forwards" element={<Forwards />} />
        <Route path="forwards/new" element={<TopologyEditor />} />
        <Route path="forwards/:id" element={<ForwardDetail />} />
        <Route path="forwards/:id/edit" element={<TopologyEditor />} />
        <Route
          path="nodes"
          element={
            <Protected admin>
              <Nodes />
            </Protected>
          }
        />
        <Route
          path="users"
          element={
            <Protected admin>
              <Users />
            </Protected>
          }
        />
        <Route
          path="invites"
          element={
            <Protected admin>
              <Invites />
            </Protected>
          }
        />
        <Route
          path="sla"
          element={
            <Protected admin>
              <SlaBoard />
            </Protected>
          }
        />
      </Route>
    </Routes>
  );
}
