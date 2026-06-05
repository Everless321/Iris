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
    // 路由 lazy split + 手动 vendor chunk：
    // 公开首页 / 只加载 main + react + antd-core，admin 重型库（reactflow / d3 / chart）按需懒加载。
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          // react + router 紧绑定，避免循环 chunk
          if (id.match(/[\\/](react|react-dom|react-router|react-router-dom|scheduler)[\\/]/)) {
            return "react-vendor";
          }
          // antd 核心组件（StatusBoard 也用 Card/Progress/Tag/Tooltip 等）
          if (id.includes("/antd/") || id.includes("@ant-design/icons")) {
            return "antd-vendor";
          }
          // antd 依赖：rc-* 老命名 + @rc-component/* 新命名 + @ant-design/cssinjs 等
          if (id.includes("/rc-") || id.includes("@rc-component") || id.includes("@ant-design")) {
            return "antd-vendor";
          }
          // dayjs / antd 也依赖
          if (id.includes("/dayjs/") || id.includes("/@ctrl/tinycolor")) {
            return "antd-vendor";
          }
          // 拓扑编辑器专用（admin 路径才加载）
          if (id.includes("@xyflow") || id.includes("/d3-")) {
            return "flow-vendor";
          }
          // 图表（admin Dashboard/SLA 才加载）
          if (id.includes("recharts") || id.includes("victory-vendor")) {
            return "chart-vendor";
          }
          // 其余统一 vendor —— 应该很小（zustand + 杂项）
          return "vendor";
        },
      },
    },
  },
});
