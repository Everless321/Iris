import { useEffect, useState } from "react";
import { Table, Card, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { api, type User } from "../lib/api";

const { Title, Text } = Typography;

export default function Users() {
  const [list, setList] = useState<User[] | null>(null);
  useEffect(() => {
    api.get<User[]>("/api/users").then(setList).catch(() => setList([]));
  }, []);

  const columns: ColumnsType<User> = [
    { title: "ID", dataIndex: "id", key: "id", width: 80, render: (id) => <Text className="num">#{id}</Text> },
    { title: "用户名", dataIndex: "username", key: "username", render: (u) => <Text strong>{u}</Text> },
    {
      title: "角色", dataIndex: "role", key: "role", width: 100,
      render: (r) => <Tag color={r === "admin" ? "gold" : "blue"}>{r}</Tag>,
    },
    {
      title: "创建时间", dataIndex: "created_at", key: "created_at", width: 200,
      render: (t) => (
        <Text className="num" type="secondary" style={{ fontSize: 12 }}>
          {t ? new Date(t).toLocaleString() : "—"}
        </Text>
      ),
    },
  ];

  return (
    <div style={{ maxWidth: 1280, margin: "0 auto" }}>
      <div style={{ marginBottom: 24 }}>
        <Title level={3} style={{ marginBottom: 4 }}>用户管理</Title>
        <Text type="secondary">查看所有注册用户与角色</Text>
      </div>

      <Card>
        <Table<User>
          rowKey="id"
          loading={list === null}
          dataSource={list ?? []}
          columns={columns}
          pagination={{ pageSize: 10, hideOnSinglePage: true }}
        />
      </Card>
    </div>
  );
}
