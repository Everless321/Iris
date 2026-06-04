import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Form, Input, Button, Card, Typography, App } from "antd";
import { LockOutlined, UserOutlined, KeyOutlined } from "@ant-design/icons";
import { useAuth } from "../lib/auth";

const { Title, Text } = Typography;

export default function Register() {
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const register = useAuth((s) => s.register);
  const { message } = App.useApp();

  async function onFinish(v: { username: string; password: string; invite_code: string }) {
    setBusy(true);
    try {
      await register(v.username, v.password, v.invite_code);
      message.success("注册成功");
      navigate("/admin", { replace: true });
    } catch (e: any) {
      message.error(e.message || "注册失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ minHeight: "100dvh", display: "flex", alignItems: "center", justifyContent: "center", padding: 24, background: "#f5f5f7" }}>
      <Card style={{ width: 420 }} bordered>
        <Title level={3} style={{ marginBottom: 4 }}>用邀请码注册</Title>
        <Text type="secondary">邀请码由管理员生成，一次性使用</Text>

        <Form layout="vertical" onFinish={onFinish} style={{ marginTop: 24 }} requiredMark={false}>
          <Form.Item
            name="username"
            label="用户名"
            rules={[{ required: true, min: 3, message: "用户名至少 3 个字符" }]}
          >
            <Input prefix={<UserOutlined />} size="large" autoFocus />
          </Form.Item>

          <Form.Item
            name="password"
            label="密码"
            rules={[{ required: true, min: 6, message: "密码至少 6 个字符" }]}
          >
            <Input.Password prefix={<LockOutlined />} size="large" />
          </Form.Item>

          <Form.Item
            name="invite_code"
            label="邀请码"
            rules={[{ required: true, message: "请输入邀请码" }]}
          >
            <Input prefix={<KeyOutlined />} size="large" style={{ fontFamily: "JetBrains Mono, monospace" }} />
          </Form.Item>

          <Form.Item style={{ marginBottom: 12 }}>
            <Button type="primary" htmlType="submit" size="large" block loading={busy}>
              创建账号
            </Button>
          </Form.Item>

          <Text type="secondary" style={{ display: "block", textAlign: "center", fontSize: 13 }}>
            已有账号？<Link to="/login">登录</Link>
          </Text>
        </Form>
      </Card>
    </div>
  );
}
