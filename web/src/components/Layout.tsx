import { useEffect, useMemo, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { Layout as AntLayout, Menu, Avatar, Dropdown, Tag, Space, theme, Tooltip, App } from "antd";
import {
  DashboardOutlined,
  SwapOutlined,
  DatabaseOutlined,
  TeamOutlined,
  TagsOutlined,
  AreaChartOutlined,
  NodeIndexOutlined,
  LogoutOutlined,
  DownOutlined,
  CloudSyncOutlined,
} from "@ant-design/icons";
import type { MenuProps } from "antd";
import { useAuth } from "../lib/auth";
import { api } from "../lib/api";

const { Sider, Header, Content } = AntLayout;

const TITLES: Record<string, string> = {
  "/admin": "概览",
  "/admin/forwards": "转发管理",
  "/admin/forwards/new": "新建转发",
  "/admin/nodes": "节点管理",
  "/admin/users": "用户管理",
  "/admin/invites": "邀请码",
  "/admin/sla": "SLA 看板",
  "/admin/latency-matrix": "节点延迟矩阵",
};

function pageTitle(path: string): string {
  if (TITLES[path]) return TITLES[path];
  if (path.startsWith("/admin/forwards/") && path.endsWith("/edit")) return "编辑转发";
  return "";
}

type UpdateCheck = {
  current: string;
  latest: string | null;
  has_update: boolean;
  error?: string;
};

export default function Layout() {
  const { user, logout } = useAuth();
  const { message, modal } = App.useApp();
  const navigate = useNavigate();
  const loc = useLocation();
  const { token } = theme.useToken();

  // M9.1 master 自检更新：每 5 分钟拉一次（后端缓存也 5 分钟）。仅 admin 可见。
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(null);
  const [upgrading, setUpgrading] = useState(false);
  useEffect(() => {
    if (user?.role !== "admin") return;
    let aborted = false;
    const check = () => {
      api.get<UpdateCheck>("/api/master/update-check")
        .then((d) => { if (!aborted) setUpdateInfo(d); })
        .catch(() => { /* 静默：网络偶发 / GitHub rate limit */ });
    };
    check();
    const t = setInterval(check, 5 * 60 * 1000);
    return () => { aborted = true; clearInterval(t); };
  }, [user?.role]);

  // M9.2 一键升级 master：fork detached install.sh，等 master 重启后页面自动 reload
  function triggerMasterUpgrade() {
    modal.confirm({
      title: "升级 master",
      content: (
        <div>
          <p>当前：<code>{updateInfo?.current}</code></p>
          <p>最新：<code>{updateInfo?.latest}</code></p>
          <p style={{ color: "#faad14", marginTop: 12 }}>
            ⚠ 升级期间 master 服务会重启约 5-10 秒，节点 gRPC 连接将短暂断开后自动重连。
          </p>
        </div>
      ),
      okText: "立即升级",
      cancelText: "取消",
      onOk: async () => {
        setUpgrading(true);
        try {
          await api.post("/api/master/upgrade");
          message.info("升级已触发，等待 master 重启...");
          // 轮询 /api/version 直到 git_hash 变化
          const start = Date.now();
          const targetHash = updateInfo?.latest;
          const poll = setInterval(async () => {
            try {
              const v = await api.get<{ git_hash: string }>("/api/version");
              if (targetHash && v.git_hash === targetHash) {
                clearInterval(poll);
                setUpgrading(false);
                message.success("✅ 升级完成，刷新页面");
                setTimeout(() => location.reload(), 1500);
              }
            } catch {
              // master 重启中，连不上，正常
            }
            if (Date.now() - start > 120000) {
              clearInterval(poll);
              setUpgrading(false);
              message.warning("升级超过 2 分钟仍未生效，请检查 /opt/iris/.master-upgrade.log");
            }
          }, 2000);
        } catch (e: unknown) {
          setUpgrading(false);
          message.error((e as Error)?.message || "升级触发失败");
        }
      },
    });
  }

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
        { key: "/admin/sla", icon: <AreaChartOutlined />, label: "SLA 看板" },
        { key: "/admin/latency-matrix", icon: <NodeIndexOutlined />, label: "延迟矩阵" }
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
          <Space size={12} align="center">
            {updateInfo && (
              updateInfo.has_update ? (
                <Tooltip title={upgrading ? "升级进行中..." : `点击升级 master 到 ${updateInfo.latest}（当前 ${updateInfo.current}）`}>
                  <Tag
                    color={upgrading ? "processing" : "warning"}
                    icon={<CloudSyncOutlined spin={upgrading} />}
                    style={{ margin: 0, cursor: upgrading ? "wait" : "pointer" }}
                    onClick={!upgrading ? triggerMasterUpgrade : undefined}
                  >
                    {upgrading ? "升级中..." : "可更新"}
                  </Tag>
                </Tooltip>
              ) : updateInfo.latest ? (
                <Tooltip title={`已是最新：${updateInfo.current}`}>
                  <Tag color="success" style={{ margin: 0, cursor: "help", fontFamily: "monospace" }}>
                    {updateInfo.current}
                  </Tag>
                </Tooltip>
              ) : null
            )}
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
          </Space>
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
