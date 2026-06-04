import { useEffect, useRef, useState } from "react";
import { Modal, Progress, Steps, Tag, Typography, App } from "antd";
import { api } from "../lib/api";

const STAGES = ["received", "preflight", "download", "verify", "dry-run", "swap", "restart"];

type CommandRow = {
  request_id: string;
  node_id: string;
  status: number;       // 0..5; 3=success 4=failed 5=rejected
  stage: string;
  detail: string;
  issued_at_ms: number;
  delivered_at_ms?: number | null;
  finished_at_ms?: number | null;
};

interface Props {
  nodeId: string | null;
  open: boolean;
  onClose: () => void;
}

export default function NodeUpgradeModal({ nodeId, open, onClose }: Props) {
  const { message } = App.useApp();
  const [requestId, setRequestId] = useState<string | null>(null);
  const [row, setRow] = useState<CommandRow | null>(null);
  const [starting, setStarting] = useState(false);
  const timerRef = useRef<number | null>(null);
  // 防止 effect 因父组件 onClose 引用变化重跑 → POST 多次 → CONFLICT
  const triggeredKeyRef = useRef<string | null>(null);
  // ref 镜像最新 callback，避免把 onClose 列入 deps
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open || !nodeId) return;
    // 同一 (open, nodeId) 只触发一次升级请求 —— 即使 effect 因依赖引用变化重跑
    const key = `${nodeId}`;
    if (triggeredKeyRef.current === key) return;
    triggeredKeyRef.current = key;
    setRequestId(null);
    setRow(null);
    setStarting(true);

    let cancelled = false;
    api
      .post<{ request_id: string; node_id: string; issued_at_ms: number }>(
        `/api/nodes/${nodeId}/upgrade`,
        { target_ref: "", expected_sha256: "" },
      )
      .then((r) => {
        if (cancelled) return;
        setRequestId(r.request_id);
      })
      .catch((e) => {
        const msg = e?.message || "升级触发失败";
        message.error(String(msg));
        onCloseRef.current();
      })
      .finally(() => setStarting(false));

    return () => {
      cancelled = true;
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, nodeId]);

  // Modal 关闭时重置 triggered key，下次打开同节点能重新触发
  useEffect(() => {
    if (!open) triggeredKeyRef.current = null;
  }, [open]);

  // 轮询命令状态直到终态
  useEffect(() => {
    if (!requestId) return;
    const poll = async () => {
      try {
        const r = await api.get<CommandRow>(`/api/commands/${requestId}`);
        setRow(r);
        if (r.status === 3 || r.status === 4 || r.status === 5) {
          if (timerRef.current !== null) {
            window.clearInterval(timerRef.current);
            timerRef.current = null;
          }
        }
      } catch {
        // ignore transient
      }
    };
    poll();
    timerRef.current = window.setInterval(poll, 2000);
    return () => {
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [requestId]);

  const status = row?.status ?? 0;
  const stage = row?.stage ?? "";
  const detail = row?.detail ?? "";
  const currentStep = STAGES.indexOf(stage);
  const done = status === 3;
  const failed = status === 4 || status === 5;
  const inProgress = !done && !failed && requestId !== null;

  return (
    <Modal
      title={`升级节点：${nodeId ?? ""}`}
      open={open}
      onCancel={onClose}
      footer={null}
      width={560}
      maskClosable={!inProgress}
      destroyOnClose
    >
      {starting && <Typography.Text>触发升级中...</Typography.Text>}
      {requestId && (
        <>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
            request_id: <code>{requestId.slice(0, 8)}</code>
          </Typography.Paragraph>
          <Steps
            direction="vertical"
            size="small"
            current={currentStep < 0 ? 0 : currentStep}
            status={failed ? "error" : done ? "finish" : "process"}
            items={STAGES.map((s) => ({ title: s, description: s === stage ? detail : "" }))}
          />
          <div style={{ marginTop: 16 }}>
            {done && <Tag color="success">升级成功 — systemd 已重启，watchdog 60s 内会验证健康</Tag>}
            {failed && (
              <Tag color="error">
                {status === 5 ? "节点拒绝（preflight 失败）" : "升级失败"}：{detail}
              </Tag>
            )}
            {inProgress && (
              <>
                <Tag color="processing">进行中</Tag>
                <Progress
                  percent={Math.max(5, Math.round(((Math.max(0, currentStep) + 1) / STAGES.length) * 100))}
                  size="small"
                  style={{ marginTop: 8 }}
                />
              </>
            )}
          </div>
        </>
      )}
    </Modal>
  );
}
