const TOKEN_KEY = "zf_token";

export const token = {
  get: () => localStorage.getItem(TOKEN_KEY),
  set: (v: string) => localStorage.setItem(TOKEN_KEY, v),
  clear: () => localStorage.removeItem(TOKEN_KEY),
};

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  const tk = token.get();
  if (tk) headers.authorization = `Bearer ${tk}`;
  const res = await fetch(path, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (res.status === 401) {
    token.clear();
    if (location.pathname !== "/login") location.replace("/login");
    throw new ApiError(401, "未授权");
  }
  if (!res.ok) {
    const msg = await res.text();
    throw new ApiError(res.status, msg || res.statusText);
  }
  if (res.status === 204) return undefined as T;
  const ct = res.headers.get("content-type") || "";
  if (ct.includes("application/json")) return res.json();
  return res.text() as T;
}

export const api = {
  get: <T>(p: string) => request<T>("GET", p),
  post: <T>(p: string, body?: unknown) => request<T>("POST", p, body),
  del: <T>(p: string) => request<T>("DELETE", p),
};

export type User = { id: number; username: string; role: "admin" | "customer"; created_at?: number };
export type AuthResp = { token: string; user: User };
export type Node = {
  id: string;
  name: string;
  addr: string;
  status: string;
  weight: number;
  health: string;
  latency_ms: number | null;
  fail_count: number;
  probe_total: number;
  probe_ok: number;
  fail_events: number;
  downtime_ms: number;
  last_seen: number | null;
  created_at: number;
};
export type HopNode = { id: string; weight: number };
export type Hop = { strategy: string; nodes: HopNode[] };
export type Forward = {
  id: number;
  name: string;
  listen_port: number;
  protocol: string;
  hops: Hop[];
  target: string;
  enabled: boolean;
  created_at: number;
};
export type Invite = {
  code: string;
  created_by: number;
  used_by: number | null;
  used_at: number | null;
  created_at: number;
};
export type Sla = {
  online: number;
  total: number;
  nodes: Array<{
    id: string;
    name: string;
    health: string;
    latency_ms: number | null;
    uptime: number;
    fail_events: number;
    downtime_ms: number;
  }>;
};
