import { FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useAuth } from "../lib/auth";

export default function Register() {
  const [u, setU] = useState("");
  const [p, setP] = useState("");
  const [code, setCode] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const register = useAuth((s) => s.register);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setErr("");
    setBusy(true);
    try {
      await register(u, p, code);
      navigate("/", { replace: true });
    } catch (e: any) {
      setErr(e.message || "注册失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center px-4">
      <form onSubmit={onSubmit} className="card w-full max-w-sm space-y-4">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Register</div>
          <h1 className="text-2xl font-semibold mt-1">用邀请码注册</h1>
        </div>
        <div>
          <label className="label">用户名 (≥3)</label>
          <input className="input" value={u} onChange={(e) => setU(e.target.value)} required autoFocus />
        </div>
        <div>
          <label className="label">密码 (≥6)</label>
          <input
            type="password"
            className="input"
            value={p}
            onChange={(e) => setP(e.target.value)}
            required
          />
        </div>
        <div>
          <label className="label">邀请码</label>
          <input className="input font-mono" value={code} onChange={(e) => setCode(e.target.value)} required />
        </div>
        {err && <div className="text-danger text-sm">{err}</div>}
        <button className="btn-primary w-full" disabled={busy}>
          {busy ? "注册中…" : "创建账号"}
        </button>
        <div className="text-xs text-mute text-center">
          已有账号？{" "}
          <Link to="/login" className="text-accent2 hover:underline">
            登录
          </Link>
        </div>
      </form>
    </div>
  );
}
