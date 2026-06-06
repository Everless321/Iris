import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Form, Input, Button, Card, Typography, App, Space } from "antd";
import { LockOutlined, UserOutlined, SafetyCertificateOutlined } from "@ant-design/icons";
import { useAuth } from "../lib/auth";

const { Title, Text } = Typography;

export default function Login() {
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const login = useAuth((s) => s.login);
  const { message } = App.useApp();

  async function onFinish(v: { username: string; password: string }) {
    setBusy(true);
    try {
      await login(v.username, v.password);
      message.success("登录成功");
      navigate("/admin", { replace: true });
    } catch (e: any) {
      message.error(e.message || "登录失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ minHeight: "100dvh", display: "grid", gridTemplateColumns: "3fr 2fr", background: "#fff" }}>
      {/* 左侧品牌叙事 */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          padding: "48px 56px",
          background:
            "linear-gradient(135deg, #1677ff 0%, #0958d9 50%, #003eb3 100%)",
          color: "#fff",
        }}
      >
        <Space size={10}>
          <img
            src="/logo.png"
            alt="Iris"
            style={{ width: 32, height: 32, borderRadius: 8, background: "#fff" }}
          />
          <span style={{ fontSize: 16, fontWeight: 600 }}>Iris</span>
        </Space>

        <div style={{ maxWidth: 480 }}>
          <Title level={1} style={{ color: "#fff", marginBottom: 16, letterSpacing: -0.5 }}>
            高性能转发平台
          </Title>
          <Text style={{ color: "rgba(255,255,255,0.85)", fontSize: 16, lineHeight: 1.7 }}>
            基于 Rust 的多级转发控制面。任意 N 跳级联、全链路 mTLS、
            每跳节点组负载均衡、故障自动切换。
          </Text>
        </div>

        <Space size={28} wrap>
          <Feature icon={<SafetyCertificateOutlined />} title="mTLS" desc="每节点独立证书" />
          <Feature icon={<UserOutlined />} title="Failover" desc="秒级故障转移" />
          <Feature icon={<LockOutlined />} title="Enrollment" desc="一次性令牌注册" />
        </Space>
      </div>

      {/* 右侧表单 */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", padding: 24 }}>
        <Card style={{ width: 380, border: "none", boxShadow: "none" }}>
          <Title level={3} style={{ marginBottom: 4 }}>欢迎回来</Title>
          <Text type="secondary">登录到 Iris 控制面</Text>

          <Form layout="vertical" onFinish={onFinish} style={{ marginTop: 28 }} requiredMark={false}>
            <Form.Item
              name="username"
              label="用户名"
              rules={[{ required: true, message: "请输入用户名" }]}
            >
              <Input prefix={<UserOutlined />} size="large" placeholder="admin" autoFocus />
            </Form.Item>

            <Form.Item
              name="password"
              label="密码"
              rules={[{ required: true, message: "请输入密码" }]}
            >
              <Input.Password prefix={<LockOutlined />} size="large" placeholder="••••••••" />
            </Form.Item>

            <Form.Item style={{ marginBottom: 12 }}>
              <Button type="primary" htmlType="submit" size="large" block loading={busy}>
                登录
              </Button>
            </Form.Item>

            <Text type="secondary" style={{ display: "block", textAlign: "center", fontSize: 13 }}>
              没有账号？<Link to="/register">用邀请码注册</Link>
            </Text>
          </Form>
        </Card>
      </div>
    </div>
  );
}

function Feature({ icon, title, desc }: { icon: React.ReactNode; title: string; desc: string }) {
  return (
    <div style={{ minWidth: 110 }}>
      <div style={{ fontSize: 20, color: "rgba(255,255,255,0.95)", marginBottom: 6 }}>{icon}</div>
      <div style={{ color: "#fff", fontWeight: 600, fontSize: 13, marginBottom: 2 }}>{title}</div>
      <div style={{ color: "rgba(255,255,255,0.7)", fontSize: 12 }}>{desc}</div>
    </div>
  );
}
