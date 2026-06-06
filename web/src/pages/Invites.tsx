import { useEffect, useState } from "react";
import { Table, Button, Card, Tag, Space, Typography, App } from "antd";
import { PlusOutlined, CopyOutlined } from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import { api, type Invite } from "../lib/api";

const { Title, Text } = Typography;

export default function Invites() {
  const [list, setList] = useState<Invite[] | null>(null);
  const { message } = App.useApp();

  const load = () => api.get<Invite[]>("/api/invites").then(setList).catch(() => setList([]));
  useEffect(() => { load(); }, []);

  async function gen() {
    try {
      await api.post("/api/invites");
      message.success("已生成邀请码");
      load();
    } catch (e: any) {
      message.error(e.message);
    }
  }

  function copy(code: string) {
    navigator.clipboard.writeText(code);
    message.success("邀请码已复制");
  }

  const columns: ColumnsType<Invite> = [
    {
      title: "邀请码", dataIndex: "code", key: "code",
      render: (c) => <Text className="num" copyable>{c}</Text>,
    },
    {
      title: "状态", dataIndex: "used_by", key: "status", width: 100,
      render: (used_by) =>
        used_by ? <Tag color="default">已使用</Tag> : <Tag color="success">可用</Tag>,
    },
    {
      title: "使用者", dataIndex: "used_by", key: "used_by", width: 120,
      render: (id) => id ? <Text className="num">#{id}</Text> : <Text type="secondary">—</Text>,
    },
    {
      title: "创建时间", dataIndex: "created_at", key: "created_at", width: 200,
      render: (t) => (
        <Text className="num" type="secondary" style={{ fontSize: 12 }}>
          {new Date(t).toLocaleString()}
        </Text>
      ),
    },
    {
      title: "操作", key: "actions", width: 100, align: "right", fixed: "right",
      render: (_, i) =>
        !i.used_by && (
          <Button type="link" size="small" icon={<CopyOutlined />} onClick={() => copy(i.code)}>
            复制
          </Button>
        ),
    },
  ];

  return (
    <div style={{ maxWidth: 1280, margin: "0 auto" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 24 }}>
        <div>
          <Title level={3} style={{ marginBottom: 4 }}>邀请码</Title>
          <Text type="secondary">生成一次性邀请码，分发给客户用于注册</Text>
        </div>
        <Button type="primary" size="large" icon={<PlusOutlined />} onClick={gen}>
          生成邀请码
        </Button>
      </div>

      <Card>
        <Table<Invite>
          rowKey="code"
          loading={list === null}
          dataSource={list ?? []}
          columns={columns}
          scroll={{ x: "max-content" }}
          pagination={{ pageSize: 10, hideOnSinglePage: true }}
        />
      </Card>
    </div>
  );
}
