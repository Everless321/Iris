import { FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useAuth } from "../lib/auth";

export default function Login() {
  const [u, setU] = useState("");
  const [p, setP] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const navigate = useNavigate();
  const login = useAuth((s) => s.login);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setErr("");
    setBusy(true);
    try {
      await login(u, p);
      navigate("/", { replace: true });
    } catch (e: any) {
      setErr(e.message || "登录失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center px-4">
      <form onSubmit={onSubmit} className="card w-full max-w-sm space-y-4">
        <div>
          <div className="text-xs uppercase tracking-[0.18em] text-mute font-mono">Sign In</div>
          <h1 className="text-2xl font-semibold mt-1">
            Zhuan<span className="text-accent">fa</span>
          </h1>
        </div>
        <div>
          <label className="label">用户名</label>
          <input className="input" value={u} onChange={(e) => setU(e.target.value)} required autoFocus />
        </div>
        <div>
          <label className="label">密码</label>
          <input
            type="password"
            className="input"
            value={p}
            onChange={(e) => setP(e.target.value)}
            required
          />
        </div>
        {err && <div className="text-danger text-sm">{err}</div>}
        <button className="btn-primary w-full" disabled={busy}>
          {busy ? "登录中…" : "登录"}
        </button>
        <div className="text-xs text-mute text-center">
          没有账号？{" "}
          <Link to="/register" className="text-accent2 hover:underline">
            用邀请码注册
          </Link>
        </div>
      </form>
    </div>
  );
}
