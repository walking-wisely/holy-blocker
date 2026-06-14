import { Activity, AlertTriangle, Database, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

type DaemonStatus = {
  state: string;
  lastHeartbeatAt: string | null;
  watchedWindows: number;
};

declare global {
  interface Window {
    holyBlocker?: {
      getDaemonStatus: () => Promise<DaemonStatus>;
    };
  }
}

function App() {
  const [status, setStatus] = useState<DaemonStatus>({
    state: "loading",
    lastHeartbeatAt: null,
    watchedWindows: 0,
  });

  useEffect(() => {
    if (window.holyBlocker) {
      void window.holyBlocker.getDaemonStatus().then(setStatus);
      return;
    }

    setStatus({
      state: "not-connected",
      lastHeartbeatAt: null,
      watchedWindows: 0,
    });
  }, []);

  return (
    <main className="appShell">
      <aside className="sidebar">
        <div className="brand">
          <ShieldCheck size={24} />
          <span>Holy Blocker</span>
        </div>
        <nav>
          <button className="navItem active" type="button">
            <Activity size={18} />
            Monitor
          </button>
          <button className="navItem" type="button">
            <Database size={18} />
            Local Data
          </button>
        </nav>
      </aside>

      <section className="content">
        <header className="pageHeader">
          <div>
            <h1>Local Monitor</h1>
            <p>Desktop control surface for the Windows daemon and on-device classifier.</p>
          </div>
          <span className="statusPill">{status.state}</span>
        </header>

        <div className="metricsGrid">
          <article className="metricCard">
            <span>Daemon</span>
            <strong>{status.state}</strong>
          </article>
          <article className="metricCard">
            <span>Watched windows</span>
            <strong>{status.watchedWindows}</strong>
          </article>
          <article className="metricCard">
            <span>Model</span>
            <strong>baseline-v0</strong>
          </article>
        </div>

        <section className="eventPanel">
          <div className="panelHeader">
            <h2>Recent Events</h2>
            <button className="iconButton" type="button" title="Flag missed item">
              <AlertTriangle size={18} />
            </button>
          </div>
          <div className="emptyState">No local daemon events have been received yet.</div>
        </section>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(<App />);
