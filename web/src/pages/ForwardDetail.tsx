import { useEffect, useState, useCallback, useRef } from "react";
import { useParams, Link } from "react-router-dom";
import {
  Card, Table, Tabs, Tag, Space, Typography, Button, Input, DatePicker,
  Empty, App, Tooltip,
} from "antd";
import { EditOutlined, ReloadOutlined, ArrowLeftOutlined } from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import dayjs, { Dayjs } from "dayjs";
import { api, type Forward, type Session, type SessionsResp } from "../lib/api";

/// SSE 实时通知：master 端 forward_sessions 表变更（任意 INSERT/UPDATE）→ EventSource
/// 触发 onRefresh。重连流程：拉 60s 单用 ticket → 开 EventSource → 失败/断开 → 重拉 ticket 再连。
/// 单用 ticket 避免 JWT 进 URL/access-log/history。debounce 300ms 抑制心跳里 burst 连击；
/// fallback 30s setInterval 兜底以防 SSE 静默死掉。
function useSessionStream(forwardId: number, onRefresh: () => void) {
  const onRefreshRef = useRef(onRefresh);
  onRefreshRef.current = onRefresh;
  useEffect(() => {
    if (!forwardId) return;
    let cancelled = false;
    let es: EventSource | null = null;
    let debounce: number | null = null;
    let reconnectTimer: number | null = null;

    const fire = () => {
      if (debounce) window.clearTimeout(debounce);
      debounce = window.setTimeout(() => onRefreshRef.current(), 300);
    };

    const connect = async () => {
      try {
        const { ticket } = await api.post<{ ticket: string }>(
          `/api/forwards/${forwardId}/sse-ticket`,
          {},
        );
        if (cancelled) return;
        const url = `/api/forwards/${forwardId}/sessions/stream?ticket=${encodeURIComponent(ticket)}`;
        es = new EventSource(url);
        es.addEventListener("refresh", fire);
        es.onerror = () => {
          // ticket 单用，断了一定要重换。close 当前 ES，3s 后重连（避免 hammer）。
          es?.close();
          es = null;
          if (cancelled) return;
          reconnectTimer = window.setTimeout(connect, 3000);
        };
      } catch {
        // ticket 拉取失败（401 / 网络）静默重试。
        if (cancelled) return;
        reconnectTimer = window.setTimeout(connect, 10000);
      }
    };

    connect();
    const fallback = window.setInterval(() => onRefreshRef.current(), 30000);

    return () => {
      cancelled = true;
      if (debounce) window.clearTimeout(debounce);
      if (reconnectTimer) window.clearTimeout(reconnectTimer);
      window.clearInterval(fallback);
      es?.close();
    };
  }, [forwardId]);
}

const { Title, Text } = Typography;
const { RangePicker } = DatePicker;

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  for (const u of units) {
    if (v < 1024) return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${u}`;
    v /= 1024;
  }
  return `${v.toFixed(2)} PB`;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem_s = s % 60;
  if (m < 60) return `${m}m ${rem_s}s`;
  const h = Math.floor(m / 60);
  const rem_m = m % 60;
  return `${h}h ${rem_m}m`;
}

function sessionColumns(active: boolean): ColumnsType<Session> {
  return [
    {
      title: "开始时间", dataIndex: "opened_at_ms", width: 170,
      render: (ms: number) => (
        <Text className="num" style={{ fontSize: 12 }}>
          {new Date(ms).toLocaleString()}
        </Text>
      ),
    },
    {
      title: "客户端", key: "client", width: 180,
      render: (_: unknown, s: Session) => (
        <Text className="num" style={{ fontSize: 12 }}>{s.client_ip}:{s.client_port}</Text>
      ),
    },
    {
      title: "目标", dataIndex: "target_addr", width: 200,
      render: (a: string) => <Text className="num" type="secondary" style={{ fontSize: 12 }}>{a}</Text>,
    },
    {
      title: "路径", dataIndex: "hops_path", width: 200,
      render: (hops: string[]) => (
        <Space size={2} wrap>
          {hops.map((h, i) => (
            <Tag key={i} color="blue" style={{ margin: 0, fontFamily: "inherit" }}>{h}</Tag>
          ))}
        </Space>
      ),
    },
    {
      title: "流量", key: "traffic", width: 140,
      render: (_: unknown, s: Session) => (
        <Tooltip title={`↑ ${s.bytes_in.toLocaleString()} B / ↓ ${s.bytes_out.toLocaleString()} B`}>
          <Space size={4} direction="vertical" style={{ fontSize: 11 }}>
            <Text className="num" style={{ fontSize: 11 }}>↑ {formatBytes(s.bytes_in)}</Text>
            <Text className="num" type="secondary" style={{ fontSize: 11 }}>↓ {formatBytes(s.bytes_out)}</Text>
          </Space>
        </Tooltip>
      ),
    },
    {
      title: "时长", key: "duration", width: 90,
      render: (_: unknown, s: Session) => {
        const end = s.closed_at_ms ?? Date.now();
        return <Text className="num" style={{ fontSize: 12 }}>{formatDuration(end - s.opened_at_ms)}</Text>;
      },
    },
    {
      title: "状态", key: "status", width: 80,
      render: (_: unknown, s: Session) => {
        if (active || s.closed_at_ms == null) {
          return <Tag color="processing" style={{ margin: 0 }}>活跃</Tag>;
        }
        const r = s.close_reason || "normal";
        const color = r === "normal" ? "success" : r === "error" ? "error" : "warning";
        return <Tag color={color} style={{ margin: 0 }}>{r}</Tag>;
      },
    },
  ];
}

function HistoryTab({ forwardId }: { forwardId: number }) {
  const { message } = App.useApp();
  const [data, setData] = useState<SessionsResp | null>(null);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [clientIp, setClientIp] = useState("");
  const [range, setRange] = useState<[Dayjs, Dayjs] | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    const params = new URLSearchParams();
    params.set("page", String(page));
    params.set("page_size", String(pageSize));
    if (clientIp.trim()) params.set("client_ip", clientIp.trim());
    if (range) {
      params.set("from", String(range[0].valueOf()));
      params.set("to", String(range[1].valueOf()));
    }
    api.get<SessionsResp>(`/api/forwards/${forwardId}/sessions?${params}`)
      .then(setData)
      .catch((e) => message.error(e.message))
      .finally(() => setLoading(false));
  }, [forwardId, page, pageSize, clientIp, range, message]);

  useEffect(() => { load(); }, [load]);
  useSessionStream(forwardId, load);

  return (
    <div>
      <Space wrap style={{ marginBottom: 12 }}>
        <RangePicker
          showTime
          value={range}
          onChange={(v) => { setRange(v as [Dayjs, Dayjs] | null); setPage(1); }}
        />
        <Input
          placeholder="按客户端 IP 搜索"
          value={clientIp}
          onChange={(e) => setClientIp(e.target.value)}
          onPressEnter={() => { setPage(1); load(); }}
          style={{ width: 200 }}
          allowClear
        />
        <Button icon={<ReloadOutlined />} onClick={load}>刷新</Button>
      </Space>
      <Table<Session>
        rowKey="id"
        loading={loading}
        dataSource={data?.sessions ?? []}
        columns={sessionColumns(false)}
        pagination={{
          current: page,
          pageSize,
          total: data?.total ?? 0,
          showSizeChanger: true,
          pageSizeOptions: [20, 50, 100, 200],
          onChange: (p, ps) => { setPage(p); setPageSize(ps); },
          showTotal: (t) => `共 ${t.toLocaleString()} 条`,
        }}
        size="small"
        locale={{ emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无连接记录" /> }}
      />
    </div>
  );
}

function ActiveTab({ forwardId }: { forwardId: number }) {
  const { message } = App.useApp();
  const [list, setList] = useState<Session[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    api.get<Session[]>(`/api/forwards/${forwardId}/sessions/active`)
      .then(setList)
      .catch((e) => message.error(e.message))
      .finally(() => setLoading(false));
  }, [forwardId, message]);

  useEffect(() => { load(); }, [load]);
  useSessionStream(forwardId, load);

  const totalBytes = list.reduce((acc, s) => acc + s.bytes_in + s.bytes_out, 0);

  return (
    <div>
      <Space style={{ marginBottom: 12 }}>
        <Tag color="processing">{list.length} 活跃</Tag>
        <Text type="secondary" style={{ fontSize: 12 }}>
          总流量 {formatBytes(totalBytes)} · 实时推送（SSE）
        </Text>
      </Space>
      <Table<Session>
        rowKey="id"
        loading={loading && list.length === 0}
        dataSource={list}
        columns={sessionColumns(true)}
        pagination={false}
        size="small"
        locale={{ emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="当前无活跃连接" /> }}
      />
    </div>
  );
}

export default function ForwardDetail() {
  const { id } = useParams();
  const forwardId = Number(id);
  const [forward, setForward] = useState<Forward | null>(null);

  useEffect(() => {
    if (!forwardId) return;
    api.get<Forward[]>("/api/forwards")
      .then((list) => setForward(list.find((f) => f.id === forwardId) ?? null))
      .catch(() => setForward(null));
  }, [forwardId]);

  if (!forwardId) return <div>Invalid forward id</div>;

  return (
    <div style={{ maxWidth: 1400, margin: "0 auto" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <Space>
          <Link to="/forwards">
            <Button type="text" icon={<ArrowLeftOutlined />} />
          </Link>
          <div>
            <Title level={3} style={{ marginBottom: 4 }}>
              {forward?.name || `Forward #${forwardId}`}
            </Title>
            {forward && (
              <Space size={6}>
                <Tag color="blue" className="num">:{forward.listen_port}</Tag>
                <Tag>{forward.protocol.toUpperCase()}</Tag>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  累计 ↑ {formatBytes(forward.bytes_in ?? 0)} / ↓ {formatBytes(forward.bytes_out ?? 0)}
                </Text>
              </Space>
            )}
          </div>
        </Space>
        <Link to={`/forwards/${forwardId}/edit`}>
          <Button icon={<EditOutlined />}>编辑</Button>
        </Link>
      </div>
      <Card>
        <Tabs
          defaultActiveKey="active"
          items={[
            { key: "active", label: "实时活跃", children: <ActiveTab forwardId={forwardId} /> },
            { key: "history", label: "历史明细", children: <HistoryTab forwardId={forwardId} /> },
          ]}
        />
      </Card>
    </div>
  );
}
