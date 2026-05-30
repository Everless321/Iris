import { FormEvent, useEffect, useState } from "react";
import { api, type Enrollment, type Node } from "../lib/api";

function HealthPill({ h }: { h: string }) {
  if (h === "healthy") return <span className="pill-ok">ok</span>;
  if (h === "unhealthy") return <span className="pill-bad">down</span>;
  return <span className="pill-warn">{h || "unknown"}</span>;
}

function InstallDialog({
  enrollment,
  onClose,
}: {
  enrollment: Enrollment;
  onClose: () => void;
}) {
  const masterUrl = `${location.protocol}//${location.host}`;
  const isLocal = /^(localhost|127\.0\.0\.1|\[::1\])(:|$)/.test(location.host);
  const isInsecure = location.protocol === "http:" && !isLocal;
  const cmd = `curl -fsSL ${masterUrl}/install.sh | bash -s -- \\
  --master ${masterUrl} \\
  --token ${enrollment.token}`;
  const expires = new Date(enrollment.expires_at).toLocaleString();
  const [copied, setCopied] = useState(false);

  function copy() {
    navigator.clipboard.writeText(cmd);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4">
      <div className="card max-w-2xl w-full space-y-4">
        <div className="flex justify-between items-start">
          <div>
            <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">
              一键安装
            </div>
            <h2 className="text-xl font-semibold mt-1">
              节点 <span className="text-accent">{enrollment.node_id}</span> 安装命令
            </h2>
          </div>
          <button className="btn-secondary !py-1.5 !px-3" onClick={onClose}>
            关闭
          </button>
        </div>
        <p className="text-sm text-dim">
          在目标服务器上 SSH 登录后，粘贴并执行这条命令。脚本会自动兑换证书、写入配置、启动节点。
        </p>
        {isInsecure && (
          <div className="rounded-md border border-danger/40 bg-danger/10 text-danger text-xs p-3 leading-relaxed">
            ⚠️ <b>不安全的链路</b>：你当前正通过 HTTP 访问 master。如果在公网执行下面命令，
            CA 私钥会以明文经过中间网络。生产环境请先给 master 套上 HTTPS（反代或直接接管 TLS），
            并在 master 设置 <span className="font-mono">ZF_REQUIRE_TLS=1</span> 强制拒绝明文 enroll。
          </div>
        )}
        <div className="relative">
          <pre className="bg-bg border border-line rounded-md p-4 text-xs font-mono text-fg overflow-x-auto whitespace-pre-wrap break-all">
            {cmd}
          </pre>
          <button
            className="btn-primary absolute top-2 right-2 !py-1.5 !px-3 text-xs"
            onClick={copy}
          >
            {copied ? "已复制 ✓" : "复制"}
          </button>
        </div>
        <div className="text-xs text-mute space-y-1">
          <div>
            令牌：<span className="font-mono text-fg">{enrollment.token}</span>
          </div>
          <div>有效期至：{expires}</div>
          <div className="text-warn">
            ⚠️ 令牌一次性使用 + 24h 失效；过期或丢失请在节点列表点「重发令牌」。
          </div>
          <div>
            首次部署前请把 <span className="font-mono">zhuanfa-node</span> 二进制放到目标机器
            <span className="font-mono"> /opt/zhuanfa/</span>，或脚本加{" "}
            <span className="font-mono">--binary &lt;路径&gt;</span> 参数。
          </div>
        </div>
      </div>
    </div>
  );
}

export default function Nodes() {
  const [list, setList] = useState<Node[]>([]);
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ id: "", name: "", addr: "", weight: 1 });
  const [err, setErr] = useState("");
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () => api.get<Node[]>("/api/nodes").then(setList).catch((e) => setErr(e.message));
  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  async function onAdd(e: FormEvent) {
    e.preventDefault();
    setErr("");
    setBusy(true);
    try {
      await api.post("/api/nodes", form);
      const tok = await api.post<Enrollment>(`/api/nodes/${form.id}/enrollment`);
      setEnrollment(tok);
      setForm({ id: "", name: "", addr: "", weight: 1 });
      setOpen(false);
      load();
    } catch (e: any) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  async function regenToken(id: string) {
    try {
      const tok = await api.post<Enrollment>(`/api/nodes/${id}/enrollment`);
      setEnrollment(tok);
    } catch (e: any) {
      alert("生成令牌失败: " + e.message);
    }
  }

  async function onDel(id: string) {
    if (!confirm(`删除节点 ${id}?`)) return;
    try {
      await api.del(`/api/nodes/${id}`);
      load();
    } catch (e: any) {
      alert(e.message);
    }
  }

  return (
    <div className="space-y-6">
      <header className="flex justify-between items-end">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Nodes</div>
          <h1 className="text-2xl font-semibold mt-1">节点管理</h1>
          <p className="text-sm text-mute mt-1">
            添加节点后会生成一键安装命令，直接 SSH 到目标服务器粘贴即可。
          </p>
        </div>
        <button className="btn-primary" onClick={() => setOpen(!open)}>
          {open ? "取消" : "+ 新增节点"}
        </button>
      </header>

      {open && (
        <form onSubmit={onAdd} className="card space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-4 gap-3">
            <div>
              <label className="label">节点 ID</label>
              <input
                className="input"
                required
                placeholder="例如 sg-1"
                value={form.id}
                onChange={(e) => setForm({ ...form, id: e.target.value })}
              />
              <div className="text-[10px] text-mute mt-1">全平台唯一短标识</div>
            </div>
            <div>
              <label className="label">名称</label>
              <input
                className="input"
                required
                placeholder="新加坡入口"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
              <div className="text-[10px] text-mute mt-1">展示用，给自己看</div>
            </div>
            <div>
              <label className="label">节点公网地址 host:port</label>
              <input
                className="input font-mono"
                required
                placeholder="1.2.3.4:7444"
                value={form.addr}
                onChange={(e) => setForm({ ...form, addr: e.target.value })}
              />
              <div className="text-[10px] text-mute mt-1">
                其他节点连接它用的地址。家宽机填映射后的外网 IP
              </div>
            </div>
            <div>
              <label className="label">权重</label>
              <input
                type="number"
                min="1"
                className="input"
                value={form.weight}
                onChange={(e) => setForm({ ...form, weight: parseInt(e.target.value) || 1 })}
              />
              <div className="text-[10px] text-mute mt-1">带宽大的填高，分流多</div>
            </div>
          </div>
          {err && <div className="text-danger text-sm">{err}</div>}
          <button className="btn-primary" disabled={busy}>
            {busy ? "创建中…" : "创建并生成安装命令"}
          </button>
        </form>
      )}

      <div className="card overflow-x-auto p-0">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wider text-mute font-mono bg-panel2">
            <tr>
              <th className="text-left px-4 py-3">ID</th>
              <th className="text-left px-4 py-3">名称</th>
              <th className="text-left px-4 py-3">地址</th>
              <th className="text-left px-4 py-3">健康</th>
              <th className="text-left px-4 py-3">延迟</th>
              <th className="text-left px-4 py-3">权重</th>
              <th className="text-left px-4 py-3">可用率</th>
              <th className="px-4 py-3"></th>
            </tr>
          </thead>
          <tbody>
            {list.map((n) => (
              <tr key={n.id} className="table-row">
                <td className="px-4 py-3 font-mono">{n.id}</td>
                <td className="px-4 py-3">{n.name}</td>
                <td className="px-4 py-3 font-mono text-dim">{n.addr}</td>
                <td className="px-4 py-3">
                  <HealthPill h={n.health} />
                </td>
                <td className="px-4 py-3 font-mono">
                  {n.latency_ms != null ? `${n.latency_ms}ms` : "—"}
                </td>
                <td className="px-4 py-3">{n.weight}</td>
                <td className="px-4 py-3 font-mono text-xs">
                  {n.probe_total > 0
                    ? `${((n.probe_ok / n.probe_total) * 100).toFixed(1)}%`
                    : "—"}
                </td>
                <td className="px-4 py-3 text-right space-x-2 whitespace-nowrap">
                  <button className="btn-secondary !py-1.5 !px-3 text-xs" onClick={() => regenToken(n.id)}>
                    重发令牌
                  </button>
                  <button className="btn-danger !py-1.5 !px-3 text-xs" onClick={() => onDel(n.id)}>
                    删除
                  </button>
                </td>
              </tr>
            ))}
            {list.length === 0 && (
              <tr>
                <td colSpan={8} className="text-mute text-center py-8">
                  暂无节点
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {enrollment && <InstallDialog enrollment={enrollment} onClose={() => setEnrollment(null)} />}
    </div>
  );
}
