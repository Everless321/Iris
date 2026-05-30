/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // 单 accent 路线：Cyan 系（系统感、运维气质，避开 AI 紫）
        accent: {
          DEFAULT: "#06b6d4", // cyan-500
          fg: "#cffafe",
          bg: "#083344",
        },
        // 中性色：Zinc（绝对中性，不偏冷不偏暖）
        ink: {
          0: "#fafafa",   // zinc-50
          1: "#e4e4e7",   // zinc-200
          2: "#a1a1aa",   // zinc-400
          3: "#71717a",   // zinc-500
          4: "#52525b",   // zinc-600
        },
        surface: {
          0: "#09090b",   // zinc-950 ← page bg
          1: "#18181b",   // zinc-900 ← panel
          2: "#27272a",   // zinc-800 ← raised
          3: "#3f3f46",   // zinc-700 ← hover
        },
        line: {
          DEFAULT: "#27272a",
          strong: "#3f3f46",
        },
        warn: "#fbbf24",  // amber-400
        danger: "#f87171",// red-400 (desaturated)
        ok: "#34d399",    // emerald-400 (only for status, not as brand)
      },
      fontFamily: {
        sans: ['Geist', '-apple-system', "system-ui", '"PingFang SC"', "sans-serif"],
        mono: ['"Geist Mono"', '"JetBrains Mono"', '"SF Mono"', "monospace"],
      },
      letterSpacing: {
        tight: "-0.01em",
        tighter: "-0.02em",
      },
      animation: {
        "fade-in": "fade-in 0.4s cubic-bezier(0.16, 1, 0.3, 1)",
        "slide-up": "slide-up 0.4s cubic-bezier(0.16, 1, 0.3, 1)",
        shimmer: "shimmer 2s linear infinite",
        "pulse-dot": "pulse-dot 2s cubic-bezier(0.4, 0, 0.6, 1) infinite",
      },
      keyframes: {
        "fade-in": { from: { opacity: 0 }, to: { opacity: 1 } },
        "slide-up": {
          from: { opacity: 0, transform: "translateY(8px)" },
          to: { opacity: 1, transform: "translateY(0)" },
        },
        shimmer: {
          from: { backgroundPosition: "200% 0" },
          to: { backgroundPosition: "-200% 0" },
        },
        "pulse-dot": {
          "0%, 100%": { opacity: 1 },
          "50%": { opacity: 0.4 },
        },
      },
    },
  },
  plugins: [],
};
