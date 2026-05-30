import { FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ArrowRight } from "@phosphor-icons/react";
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
    <div className="min-h-[100dvh] flex items-center justify-center px-8">
      <form onSubmit={onSubmit} className="w-full max-w-[320px] animate-slide-up">
        <p className="eyebrow mb-3">create account</p>
        <h2 className="text-2xl font-medium tracking-tight mb-8">用邀请码注册</h2>

        <div className="space-y-6">
          <div>
            <label className="label">Username <span className="text-ink-3 normal-case">（≥ 3）</span></label>
            <input className="field" value={u} onChange={(e) => setU(e.target.value)} required autoFocus />
          </div>
          <div>
            <label className="label">Password <span className="text-ink-3 normal-case">（≥ 6）</span></label>
            <input type="password" className="field" value={p} onChange={(e) => setP(e.target.value)} required />
          </div>
          <div>
            <label className="label">Invite code</label>
            <input className="field font-mono text-xs" value={code} onChange={(e) => setCode(e.target.value)} required />
          </div>

          {err && (
            <p className="text-danger text-sm flex items-start gap-1.5">
              <span className="block w-1 self-stretch bg-danger rounded-full mt-0.5" />
              {err}
            </p>
          )}

          <button className="btn-primary w-full justify-between" disabled={busy}>
            <span>{busy ? "Creating…" : "Create account"}</span>
            <ArrowRight size={14} />
          </button>
        </div>

        <p className="text-xs text-ink-3 mt-8 text-center">
          已有账号？{" "}
          <Link to="/login" className="btn-link">登录</Link>
        </p>
      </form>
    </div>
  );
}
