import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:7080",
      "/metrics": "http://127.0.0.1:7080",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // 不手动 manualChunks —— 之前激进按包名拆 vendor 触发循环依赖,
    // React 模块初始化时机错乱导致 useState undefined。
    // 完全靠 React.lazy() 让 vite 自动按动态 import 边界拆 chunk:
    //   - eager: react / antd / vendor → 公开首页 / 加载
    //   - lazy: 每个 admin 页 + 其专属依赖（recharts/xyflow）独立 chunk
    chunkSizeWarningLimit: 800,
  },
});
