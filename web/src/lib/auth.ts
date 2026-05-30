import { create } from "zustand";
import { api, token, type User, type AuthResp } from "./api";

type AuthState = {
  user: User | null;
  loading: boolean;
  init: () => Promise<void>;
  login: (u: string, p: string) => Promise<void>;
  register: (u: string, p: string, code: string) => Promise<void>;
  logout: () => void;
};

export const useAuth = create<AuthState>((set) => ({
  user: null,
  loading: true,
  init: async () => {
    if (!token.get()) return set({ user: null, loading: false });
    try {
      const u = await api.get<User>("/api/me");
      set({ user: u, loading: false });
    } catch {
      token.clear();
      set({ user: null, loading: false });
    }
  },
  login: async (username, password) => {
    const r = await api.post<AuthResp>("/api/auth/login", { username, password });
    token.set(r.token);
    set({ user: r.user });
  },
  register: async (username, password, invite_code) => {
    const r = await api.post<AuthResp>("/api/auth/register", { username, password, invite_code });
    token.set(r.token);
    set({ user: r.user });
  },
  logout: () => {
    token.clear();
    set({ user: null });
  },
}));
