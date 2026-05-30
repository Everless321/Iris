import { FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ArrowRight, ShieldCheck, Lock, GraphicsCard } from "@phosphor-icons/react";
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
    <div className="min-h-[100dvh] grid grid-cols-1 md:grid-cols-[3fr_2fr]">
      {/* —— 左侧：品牌叙事（非对称大留白）—— */}
      <div className="hidden md:flex flex-col justify-between px-16 py-12 border-r border-line">
        <div className="flex items-center gap-2">
          <div className="w-7 h-7 rounded-md bg-ink-0 text-surface-0 flex items-center justify-center font-mono text-xs font-bold">
            z
          </div>
          <span className="text-sm tracking-tight">zhuanfa</span>
        </div>

        <div className="space-y-6 max-w-[44ch]">
          <p className="eyebrow">control plane</p>
          <h1 className="text-4xl tracking-tighter leading-[1.05] font-medium">
            高性能转发平台<br />
            <span className="text-ink-3">智能调度 · SLA 承诺</span>
          </h1>
          <p className="text-ink-2 text-sm leading-relaxed">
            基于 Rust 的多级转发控制面。任意 N 跳级联、全链路 mTLS、
            每跳节点组负载均衡、故障自动切换。
          </p>
        </div>

        <div className="grid grid-cols-3 gap-x-8 text-xs">
          <div className="space-y-1">
            <ShieldCheck size={16} className="text-accent-fg" />
            <p className="text-ink-1 mt-2">mTLS</p>
            <p className="text-ink-3">每节点独立证书</p>
          </div>
          <div className="space-y-1">
            <GraphicsCard size={16} className="text-accent-fg" />
            <p className="text-ink-1 mt-2">Failover</p>
            <p className="text-ink-3">秒级故障转移</p>
          </div>
          <div className="space-y-1">
            <Lock size={16} className="text-accent-fg" />
            <p className="text-ink-1 mt-2">Enrollment</p>
            <p className="text-ink-3">一次性令牌</p>
          </div>
        </div>
      </div>

      {/* —— 右侧：表单（无卡片，靠空间分组）—— */}
      <div className="flex items-center justify-center px-8 py-12">
        <form onSubmit={onSubmit} className="w-full max-w-[320px] animate-slide-up">
          <p className="eyebrow mb-3">sign in</p>
          <h2 className="text-2xl font-medium tracking-tight mb-8">
            Welcome back
          </h2>

          <div className="space-y-6">
            <div>
              <label className="label">Username</label>
              <input
                className="field"
                value={u}
                onChange={(e) => setU(e.target.value)}
                required
                autoFocus
                autoComplete="username"
              />
            </div>

            <div>
              <label className="label">Password</label>
              <input
                type="password"
                className="field"
                value={p}
                onChange={(e) => setP(e.target.value)}
                required
                autoComplete="current-password"
              />
            </div>

            {err && (
              <p className="text-danger text-sm flex items-start gap-1.5">
                <span className="block w-1 self-stretch bg-danger rounded-full mt-0.5" />
                {err}
              </p>
            )}

            <button className="btn-primary w-full justify-between" disabled={busy}>
              <span>{busy ? "Signing in…" : "Sign in"}</span>
              <ArrowRight size={14} />
            </button>
          </div>

          <p className="text-xs text-ink-3 mt-8 text-center">
            没有账号？{" "}
            <Link to="/register" className="btn-link">
              用邀请码注册
            </Link>
          </p>
        </form>
      </div>
    </div>
  );
}
