import { FormEvent, useEffect, useState } from "react";
import { ArrowUpRight, Copy, Plus, Trash, X, Warning, Check } from "@phosphor-icons/react";
import { api, type Enrollment, type Node } from "../lib/api";

function StatusDot({ h }: { h: string }) {
  if (h === "healthy") return <span className="dot-ok" />;
  if (h === "unhealthy") return <span className="dot-bad" />;
  return <span className="dot-unknown" />;
}

function HealthCell({ h }: { h: string }) {
  const cls = h === "healthy" ? "tag-ok" : h === "unhealthy" ? "tag-bad" : "tag-muted";
  return (
    <span className={`inline-flex items-center ${cls}`}>
      <StatusDot h={h} /> {h === "healthy" ? "ok" : h === "unhealthy" ? "down" : "unknown"}
    </span>
  );
}

function InstallDialog({ enrollment, onClose }: { enrollment: Enrollment; onClose: () => void }) {
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
    <div className="fixed inset-0 bg-surface-0/80 backdrop-blur-sm flex items-center justify-center z-50 p-4 animate-fade-in">
      <div className="w-full max-w-2xl bg-surface-1 border border-line rounded-lg p-8 space-y-6 animate-slide-up">
        <header className="flex justify-between items-start">
          <div>
            <p className="eyebrow">enrollment</p>
            <h2 className="text-lg tracking-tight font-medium mt-1">
              Install command for <span className="font-mono text-accent-fg">{enrollment.node_id}</span>
            </h2>
          </div>
          <button className="text-ink-3 hover:text-ink-0 transition-colors" onClick={onClose}>
            <X size={18} />
          </button>
        </header>

        <p className="text-sm text-ink-2 leading-relaxed">
          SSH 到目标服务器后粘贴执行。脚本会自动兑换证书、写入配置、启动节点。
        </p>

        {isInsecure && (
          <div className="border-l-2 border-danger pl-4 py-2 text-xs text-danger leading-relaxed flex gap-2 items-start">
            <Warning size={14} className="shrink-0 mt-0.5" />
            <div>
              <strong className="block mb-1">Insecure channel.</strong>
              你正通过 HTTP 访问 master。生产部署请套上 HTTPS 并设置{" "}
              <span className="font-mono">ZF_REQUIRE_TLS=1</span>，否则证书私钥可能在中间链路被截获。
            </div>
          </div>
        )}

        <div className="relative">
          <pre className="bg-surface-0 border border-line rounded-md p-4 text-xs font-mono text-ink-1 overflow-x-auto whitespace-pre-wrap break-all leading-relaxed">
            {cmd}
          </pre>
          <button className="btn-outline btn-sm absolute top-2 right-2" onClick={copy}>
            {copied ? <Check size={12} /> : <Copy size={12} />}
            <span>{copied ? "Copied" : "Copy"}</span>
          </button>
        </div>

        <dl className="grid grid-cols-2 gap-y-2 text-xs">
          <dt className="text-ink-3">Token</dt>
          <dd className="font-mono text-ink-1 break-all">{enrollment.token}</dd>
          <dt className="text-ink-3">Expires</dt>
          <dd className="font-mono text-ink-1">{expires}</dd>
        </dl>

        <p className="text-xs text-ink-3 leading-relaxed border-t border-line pt-4">
          令牌一次性使用 + 24h 失效；过期请在节点行点 <span className="font-mono">Resend</span>。
          首次部署请把 <span className="font-mono">zhuanfa-node</span> 二进制放到目标
          <span className="font-mono"> /opt/zhuanfa/</span>，或加 <span className="font-mono">--binary &lt;路径&gt;</span>。
        </p>
      </div>
    </div>
  );
}

export default function Nodes() {
  const [list, setList] = useState<Node[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ id: "", name: "", addr: "", weight: 1 });
  const [err, setErr] = useState("");
  const [enrollment, setEnrollment] = useState<Enrollment | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () =>
    api.get<Node[]>("/api/nodes").then(setList).catch((e) => {
      setList([]);
      setErr(e.message);
    });

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
      setAdding(false);
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
    if (!confirm(`Delete node ${id}?`)) return;
    try {
      await api.del(`/api/nodes/${id}`);
      load();
    } catch (e: any) {
      alert(e.message);
    }
  }

  return (
    <div className="px-8 py-10 max-w-[1280px] mx-auto space-y-8 animate-slide-up">
      <header className="flex items-end justify-between">
        <div>
          <p className="eyebrow">nodes</p>
          <h1 className="text-2xl tracking-tight font-medium mt-1">Nodes</h1>
          <p className="text-xs text-ink-3 mt-2 max-w-[56ch]">
            添加节点后会生成一键安装命令。SSH 到目标服务器粘贴即可，每个节点拿到独立 mTLS 证书。
          </p>
        </div>
        <button className="btn-primary" onClick={() => setAdding((v) => !v)}>
          {adding ? <X size={14} /> : <Plus size={14} />}
          <span>{adding ? "Cancel" : "New node"}</span>
        </button>
      </header>

      {adding && (
        <form
          onSubmit={onAdd}
          className="border border-line rounded-md p-6 space-y-5 bg-surface-1/40 animate-slide-up"
        >
          <div className="grid grid-cols-1 md:grid-cols-4 gap-5">
            <Field
              label="Node ID"
              hint="全平台唯一短标识"
              value={form.id}
              onChange={(v) => setForm({ ...form, id: v })}
              placeholder="sg-1"
              required
            />
            <Field
              label="Name"
              hint="展示用"
              value={form.name}
              onChange={(v) => setForm({ ...form, name: v })}
              placeholder="新加坡入口"
              required
            />
            <Field
              label="Public address"
              hint="其它节点连接它的 host:port"
              value={form.addr}
              onChange={(v) => setForm({ ...form, addr: v })}
              placeholder="1.2.3.4:7444"
              mono
              required
            />
            <div>
              <label className="label">Weight</label>
              <input
                type="number"
                min={1}
                className="field-box num"
                value={form.weight}
                onChange={(e) => setForm({ ...form, weight: parseInt(e.target.value) || 1 })}
              />
              <p className="text-[10px] text-ink-3 mt-1.5">带宽大的填高</p>
            </div>
          </div>
          {err && <p className="text-danger text-sm">{err}</p>}
          <button className="btn-primary" disabled={busy}>
            {busy ? "Creating…" : "Create & generate install command"}
          </button>
        </form>
      )}

      {list === null ? (
        <Skeleton />
      ) : list.length === 0 ? (
        <Empty />
      ) : (
        <div className="border-t border-line">
          <div className="grid grid-cols-[0.8fr_1fr_1.5fr_0.7fr_0.6fr_0.5fr_0.7fr_auto] gap-x-4 py-2 px-2 table-h border-b border-line">
            <span>ID</span>
            <span>Name</span>
            <span>Address</span>
            <span>Health</span>
            <span>Latency</span>
            <span>Weight</span>
            <span>Uptime</span>
            <span />
          </div>
          <ul className="divide-y divide-line">
            {list.map((n) => (
              <li key={n.id} className="row-hover group">
                <div className="grid grid-cols-[0.8fr_1fr_1.5fr_0.7fr_0.6fr_0.5fr_0.7fr_auto] gap-x-4 items-center py-3 px-2">
                  <span className="num text-sm">{n.id}</span>
                  <span className="text-sm truncate">{n.name}</span>
                  <span className="num text-xs text-ink-3 truncate">{n.addr}</span>
                  <HealthCell h={n.health} />
                  <span className="num text-xs text-ink-2">
                    {n.latency_ms != null ? `${n.latency_ms}ms` : "—"}
                  </span>
                  <span className="num text-xs text-ink-2">{n.weight}</span>
                  <span className="num text-xs text-ink-2">
                    {n.probe_total > 0
                      ? `${((n.probe_ok / n.probe_total) * 100).toFixed(1)}%`
                      : "—"}
                  </span>
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button className="btn-outline btn-sm" onClick={() => regenToken(n.id)}>
                      Resend
                    </button>
                    <button className="btn-danger btn-sm" onClick={() => onDel(n.id)} title="删除">
                      <Trash size={12} />
                    </button>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {enrollment && <InstallDialog enrollment={enrollment} onClose={() => setEnrollment(null)} />}
    </div>
  );
}

function Field({
  label, hint, value, onChange, placeholder, required, mono,
}: {
  label: string; hint?: string; value: string;
  onChange: (v: string) => void; placeholder?: string; required?: boolean; mono?: boolean;
}) {
  return (
    <div>
      <label className="label">{label}</label>
      <input
        className={`field-box ${mono ? "font-mono" : ""}`}
        required={required}
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      {hint && <p className="text-[10px] text-ink-3 mt-1.5">{hint}</p>}
    </div>
  );
}

function Skeleton() {
  return (
    <ul className="border-t border-line divide-y divide-line">
      {Array.from({ length: 3 }).map((_, i) => (
        <li key={i} className="py-4 px-2">
          <div className="skel h-4 w-1/4 mb-2" />
          <div className="skel h-3 w-2/3" />
        </li>
      ))}
    </ul>
  );
}

function Empty() {
  return (
    <div className="border border-dashed border-line rounded-md px-6 py-16 text-center">
      <p className="text-sm text-ink-1 font-medium">No nodes yet</p>
      <p className="text-xs text-ink-3 mt-1 max-w-[44ch] mx-auto">
        添加第一个节点，平台会生成一行安装命令。SSH 到目标服务器粘贴执行就行。
      </p>
    </div>
  );
}
