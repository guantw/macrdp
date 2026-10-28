import { useEffect, useRef, useState } from "react";
import { AlertDialog } from "@base-ui/react/alert-dialog";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Settings,
  LogOut,
  FileText,
  Play,
  Square,
  Monitor,
  Loader2,
} from "lucide-react";
import { useServerStatus } from "../hooks/useServerStatus";
import { useMetrics } from "../hooks/useMetrics";
import { useConnections } from "../hooks/useConnections";
import { api } from "../lib/ipc";
import { formatBytes, formatDuration } from "../lib/format";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";

const statusDotColor: Record<string, string> = {
  running: "bg-green",
  starting: "bg-accent",
  stopping: "bg-accent",
  stopped: "bg-muted-foreground",
  error: "bg-destructive",
};

function Popover() {
  const { status, startServer, stopServer } = useServerStatus();
  const { metrics, stale } = useMetrics();
  const connections = useConnections();
  const [quitting, setQuitting] = useState(false);
  const [quitError, setQuitError] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;

    // Measure the content, not the viewport, so empty and populated lists both fit.
    const observer = new ResizeObserver(() => {
      const { width, height } = content.getBoundingClientRect();
      getCurrentWindow()
        .setSize(new LogicalSize(Math.ceil(width), Math.ceil(height)))
        .catch((err) => console.error("Failed to resize popover:", err));
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, []);

  const metricsAvailable = metrics && !stale;
  const transitioning = status.state === "starting" || status.state === "stopping";
  const metricItems = [
    { label: "FPS", value: metricsAvailable ? metrics.fps.toFixed(0) : "--" },
    { label: "Mbps", value: metricsAvailable ? (metrics.bitrate_kbps / 1000).toFixed(1) : "--" },
    { label: "RTT (ms)", value: metricsAvailable ? metrics.latency_ms.toFixed(1) : "--" },
    { label: "编码 (ms)", value: metricsAvailable ? metrics.encode_ms.toFixed(1) : "--" },
    { label: "网络 (ms)", value: metricsAvailable ? metrics.net_ms.toFixed(1) : "--" },
    { label: "流量", value: metricsAvailable ? formatBytes(metrics.bytes_sent) : "--" },
  ];

  const handleToggle = async () => {
    try {
      if (status.running) {
        await stopServer();
      } else {
        await startServer();
      }
    } catch (err) {
      console.error("Failed to toggle server:", err);
    }
  };

  const handleQuit = async () => {
    setQuitting(true);
    setQuitError(null);
    try {
      await api.quitApp();
    } catch (err) {
      console.error("Failed to quit:", err);
      setQuitError("退出失败，请重试");
    } finally {
      setQuitting(false);
    }
  };

  const handleShowMain = () => {
    api.showMainWindow();
  };

  const handleShowLogs = () => {
    api.showMainWindow();
  };

  const displayedConnections = connections.slice(0, 3);
  const extraCount = connections.length - 3;

  return (
    <div ref={contentRef} className="bg-bg/95 backdrop-blur-xl rounded-xl overflow-hidden shadow-2xl">
      <div className="flex flex-col p-3.5 gap-3">
        {/* Header row */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div
              className={`h-2 w-2 rounded-full ${statusDotColor[status.state] ?? "bg-muted-foreground"}`}
            />
            <span className="text-sm font-semibold text-foreground">
              macrdp
            </span>
            {status.state === "running" && status.uptime_secs > 0 && (
              <span className="text-xs text-muted-foreground">
                {formatDuration(status.uptime_secs)}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={handleShowMain}
              title="设置"
            >
              <Settings className="h-3.5 w-3.5" />
            </Button>
            <AlertDialog.Root onOpenChange={() => setQuitError(null)}>
              <AlertDialog.Trigger
                render={<Button variant="destructive" size="icon-xs" title="退出" aria-label="退出" />}
              >
                <LogOut className="h-3.5 w-3.5" />
              </AlertDialog.Trigger>
              <AlertDialog.Portal>
                <AlertDialog.Backdrop className="fixed inset-0 z-50 bg-black/25" />
                <AlertDialog.Popup className="fixed left-1/2 top-1/2 z-50 w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card p-4 shadow-lg">
                  <AlertDialog.Title className="text-sm font-semibold">
                    确定退出 macrdp？
                  </AlertDialog.Title>
                  <AlertDialog.Description className="mt-2 text-xs text-muted-foreground">
                    退出将停止服务并断开当前连接。
                  </AlertDialog.Description>
                  {quitError && (
                    <p role="alert" className="mt-2 text-xs text-destructive">{quitError}</p>
                  )}
                  <div className="mt-4 flex justify-end gap-2">
                    <AlertDialog.Close render={<Button variant="outline" size="sm" disabled={quitting} />}>
                      取消
                    </AlertDialog.Close>
                    <Button variant="destructive" size="sm" disabled={quitting} onClick={handleQuit}>
                      {quitting ? "正在退出…" : "退出"}
                    </Button>
                  </div>
                </AlertDialog.Popup>
              </AlertDialog.Portal>
            </AlertDialog.Root>
          </div>
        </div>

        {/* Two rows leave room for formatted values and units at 300px wide. */}
        <dl className="grid grid-cols-3 gap-1.5">
          {metricItems.map(({ label, value }) => (
            <div key={label} className="flex min-w-0 flex-col items-center rounded-md bg-muted/50 px-1 py-2">
              <dd title={value} className="w-full truncate text-center text-sm font-semibold tabular-nums text-foreground">
                {value}
              </dd>
              <dt className="mt-0.5 text-[10px] text-muted-foreground">{label}</dt>
            </div>
          ))}
        </dl>

        {/* Connection list */}
        <div>
          <div className="mb-1.5 text-xs text-muted-foreground">
            连接 ({connections.length})
          </div>
          {connections.length === 0 ? (
            <div className="text-center text-xs text-muted-foreground py-2">
              暂无连接
            </div>
          ) : (
            <div className="space-y-1">
              {displayedConnections.map((conn) => {
                const connectedAt = new Date(conn.connected_at);
                const durationSecs = Math.floor(
                  (Date.now() - connectedAt.getTime()) / 1000,
                );
                return (
                  <div
                    key={`${conn.client_ip}-${conn.connected_at}`}
                    className="flex items-center justify-between text-xs px-2 py-1.5 rounded-md bg-muted/30"
                  >
                    <div className="flex items-center gap-1.5 min-w-0">
                      <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-green" />
                      <span className="truncate text-foreground font-medium">
                        {conn.client_name || conn.client_ip}
                      </span>
                      <span className="shrink-0 text-muted-foreground">
                        {formatDuration(durationSecs)}
                      </span>
                    </div>
                    <span className="shrink-0 ml-2 text-muted-foreground">
                      {formatBytes(conn.bytes_total)}
                    </span>
                  </div>
                );
              })}
              {extraCount > 0 && (
                <button
                  type="button"
                  onClick={handleShowMain}
                  className="w-full text-center text-xs text-primary hover:underline py-0.5"
                >
                  +{extraCount} 更多
                </button>
              )}
            </div>
          )}
        </div>

        <Separator />

        {/* Action buttons row */}
        <div className="grid grid-cols-3 gap-2">
          <Button
            variant={status.running ? "destructive" : "default"}
            size="sm"
            disabled={transitioning}
            onClick={handleToggle}
            className="w-full disabled:bg-muted disabled:text-muted-foreground"
          >
            {transitioning ? (
              <><Loader2 className="h-3 w-3 animate-spin" /> {status.state === "starting" ? "启动中..." : "停止中..."}</>
            ) : status.running ? (
              <><Square className="h-3 w-3" /> 停止</>
            ) : (
              <><Play className="h-3 w-3" /> 启动</>
            )}
          </Button>
          <Button
            size="sm"
            onClick={handleShowMain}
            className="w-full"
          >
            <Monitor className="h-3 w-3" />
            主窗口
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleShowLogs}
            className="w-full"
          >
            <FileText className="h-3 w-3" />
            日志
          </Button>
        </div>
      </div>
    </div>
  );
}

export default Popover;
