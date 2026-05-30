/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        accent: "#7cf3a0",
        accent2: "#5ad6ff",
        bg: "#0b0d10",
        panel: "#12161c",
        panel2: "#181d25",
        line: "#222933",
        fg: "#e6e9ee",
        dim: "#8a93a3",
        mute: "#5b6470",
        warn: "#ffb454",
        danger: "#ff6b6b",
      },
      fontFamily: {
        mono: ['"SF Mono"', '"JetBrains Mono"', "Menlo", "monospace"],
      },
    },
  },
  plugins: [],
};
