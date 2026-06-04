import { useMemo } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { Layout as AntLayout, Menu, Avatar, Dropdown, Tag, Space, theme } from "antd";
import {
  DashboardOutlined,
  SwapOutlined,
  DatabaseOutlined,
  TeamOutlined,
  TagsOutlined,
  AreaChartOutlined,
  LogoutOutlined,
  DownOutlined,
} from "@ant-design/icons";
import type { MenuProps } from "antd";
import { useAuth } from "../lib/auth";

const { Sider, Header, Content } = AntLayout;

const TITLES: Record<string, string> = {
  "/admin": "概览",
  "/admin/forwards": "转发管理",
  "/admin/forwards/new": "新建转发",
  "/admin/nodes": "节点管理",
  "/admin/users": "用户管理",
  "/admin/invites": "邀请码",
  "/admin/sla": "SLA 看板",
};

function pageTitle(path: string): string {
  if (TITLES[path]) return TITLES[path];
  if (path.startsWith("/admin/forwards/") && path.endsWith("/edit")) return "编辑转发";
  return "";
}

export default function Layout() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const loc = useLocation();
  const { token } = theme.useToken();

  const menuItems = useMemo<MenuProps["items"]>(() => {
    const base = [
      { key: "/admin", icon: <DashboardOutlined />, label: "概览" },
      { key: "/admin/forwards", icon: <SwapOutlined />, label: "我的转发" },
    ];
    if (user?.role === "admin") {
      base.push(
        { key: "/admin/nodes", icon: <DatabaseOutlined />, label: "节点" },
        { key: "/admin/users", icon: <TeamOutlined />, label: "用户" },
        { key: "/admin/invites", icon: <TagsOutlined />, label: "邀请码" },
        { key: "/admin/sla", icon: <AreaChartOutlined />, label: "SLA 看板" }
      );
    }
    return base;
  }, [user?.role]);

  const userMenu: MenuProps["items"] = [
    {
      key: "logout",
      icon: <LogoutOutlined />,
      label: "退出登录",
      onClick: () => {
        logout();
        navigate("/login");
      },
    },
  ];

  // 选中项匹配
  const selectedKey = useMemo(() => {
    if (loc.pathname.startsWith("/admin/forwards")) return "/admin/forwards";
    // /admin 索引页（概览）单独匹配，避免被 /admin/nodes 等子路径覆盖
    if (loc.pathname === "/admin" || loc.pathname === "/admin/") return "/admin";
    return loc.pathname;
  }, [loc.pathname]);

  return (
    <AntLayout style={{ minHeight: "100dvh" }}>
      <Sider
        width={220}
        style={{
          borderRight: `1px solid ${token.colorBorderSecondary}`,
          background: "#fff",
        }}
        breakpoint="md"
        collapsedWidth={64}
      >
        <div
          style={{
            height: 56,
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: "0 18px",
            borderBottom: `1px solid ${token.colorBorderSecondary}`,
          }}
        >
          <img
            src="/logo.png"
            alt="Iris"
            style={{ width: 28, height: 28, borderRadius: 6 }}
          />
          <span style={{ fontSize: 15, fontWeight: 600, letterSpacing: 0.2 }}>Iris</span>
        </div>
        <Menu
          mode="inline"
          selectedKeys={[selectedKey]}
          items={menuItems}
          onClick={({ key }) => navigate(key)}
          style={{ borderRight: 0, padding: "8px 8px" }}
        />
      </Sider>

      <AntLayout>
        <Header
          style={{
            background: "#fff",
            borderBottom: `1px solid ${token.colorBorderSecondary}`,
            padding: "0 24px",
            height: 56,
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <div style={{ fontSize: 15, fontWeight: 500 }}>{pageTitle(loc.pathname)}</div>
          <Dropdown menu={{ items: userMenu }} placement="bottomRight">
            <Space style={{ cursor: "pointer" }} size={8}>
              <Avatar size={28} style={{ background: "#1677ff", fontSize: 12 }}>
                {user?.username?.[0]?.toUpperCase()}
              </Avatar>
              <span style={{ fontSize: 13 }}>{user?.username}</span>
              <Tag color={user?.role === "admin" ? "gold" : "blue"} style={{ margin: 0 }}>
                {user?.role}
              </Tag>
              <DownOutlined style={{ fontSize: 10, color: "#999" }} />
            </Space>
          </Dropdown>
        </Header>

        <Content
          style={{
            padding: "24px 32px",
            background: "#f5f5f7",
            overflowY: "auto",
          }}
        >
          <Outlet />
        </Content>
      </AntLayout>
    </AntLayout>
  );
}
