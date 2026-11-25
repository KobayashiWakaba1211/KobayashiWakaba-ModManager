import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("React 渲染错误:", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            padding: 24,
            color: "#ff8fa3",
            background: "#12121a",
            height: "100vh",
            fontFamily: "monospace",
            overflow: "auto",
          }}
        >
          <h2 style={{ marginBottom: 12 }}>⚠️ 界面渲染出错</h2>
          <pre style={{ whiteSpace: "pre-wrap", color: "#f87171", fontSize: 13 }}>
            {this.state.error.message}
            {"\n\n"}
            {this.state.error.stack}
          </pre>
          <button
            onClick={() => this.setState({ error: null })}
            style={{
              marginTop: 16,
              padding: "8px 16px",
              background: "#ff8fa3",
              color: "#fff",
              border: "none",
              borderRadius: 8,
              cursor: "pointer",
            }}
          >
            重试
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);

// 诊断标志：挂载完成
(window as unknown as { __app_loaded: boolean }).__app_loaded = true;
