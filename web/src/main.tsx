import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { ConfigProvider, App as AntApp, theme } from "antd";
import zhCN from "antd/locale/zh_CN";
import App from "./App";
import "./index.css";

const themeToken = {
  token: {
    colorPrimary: "#1677ff",
    colorBgLayout: "#f5f5f7",
    colorBgContainer: "#ffffff",
    colorBorderSecondary: "#f0f0f0",
    borderRadius: 8,
    borderRadiusLG: 10,
    fontFamily:
      '-apple-system, "PingFang SC", "Microsoft YaHei", system-ui, "Helvetica Neue", Arial, sans-serif',
    fontSize: 14,
    lineHeight: 1.6,
  },
  components: {
    Layout: { siderBg: "#ffffff", headerBg: "#ffffff" },
    Menu: { itemBg: "transparent", itemSelectedBg: "#e6f4ff" },
    Card: { headerBg: "transparent" },
    Modal: { titleFontSize: 18 },
    Input: { paddingBlock: 8, paddingInline: 12 },
    Select: { optionSelectedBg: "#e6f4ff" },
    Table: { headerBg: "#fafafa", rowHoverBg: "#fafafa" },
    Statistic: { titleFontSize: 12 },
  },
  algorithm: theme.defaultAlgorithm,
};

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ConfigProvider locale={zhCN} theme={themeToken}>
      <AntApp>
        <BrowserRouter>
          <App />
        </BrowserRouter>
      </AntApp>
    </ConfigProvider>
  </React.StrictMode>
);
