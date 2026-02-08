import { useEffect, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { ArrowLeft, Database, Info, Calendar, Video, Tag } from "lucide-react";

interface RunInfo {
  name: string;
  metadata: {
    original_name: string;
    created_at: string;
    run_id: string;
  };
}

const RunDetailsPage: React.FC = () => {
  const { id } = useParams();
  const navigate = useNavigate();
  const [run, setRun] = useState<RunInfo | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch("/api/runs")
      .then((res) => res.json())
      .then((data: RunInfo[]) => {
        const found = data.find((r) => r.metadata.run_id === id);
        setRun(found || null);
        setLoading(false);
      });
  }, [id]);

  if (loading) {
    return <div className="container empty-state">Loading run details...</div>;
  }

  if (!run) {
    return (
      <div className="container empty-state">
        <h2>Run Not Found</h2>
        <button
          onClick={() => navigate("/")}
          className="btn btn-secondary"
          style={{ marginTop: "1rem" }}
        >
          Back to Dashboard
        </button>
      </div>
    );
  }

  return (
    <div className="container">
      <button
        onClick={() => navigate("/")}
        className="btn btn-secondary"
        style={{ marginBottom: "2rem" }}
      >
        <ArrowLeft size={18} />
        Back to Dashboard
      </button>

      <div className="nav" style={{ border: "none", marginBottom: "1rem" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "1rem" }}>
          <div
            style={{
              width: "48px",
              height: "48px",
              background: "rgba(52, 211, 153, 0.1)",
              borderRadius: "12px",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Database size={24} color="#34d399" />
          </div>
          <div>
            <h1>{run.metadata.run_id}</h1>
            <p style={{ color: "var(--text-secondary)" }}>
              Analysis Session Details
            </p>
          </div>
        </div>
        <div className="badge badge-run">Processed</div>
      </div>

      <div className="grid grid-cols-2">
        <div className="glass-card">
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.5rem",
              marginBottom: "1.5rem",
              borderBottom: "1px solid var(--border-color)",
              paddingBottom: "0.75rem",
            }}
          >
            <Info size={18} color="var(--accent-secondary)" />
            <h3 style={{ fontSize: "1rem" }}>Metadata</h3>
          </div>

          <div className="grid" style={{ gap: "1rem" }}>
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "0.5rem",
                  color: "var(--text-muted)",
                  fontSize: "0.875rem",
                }}
              >
                <Tag size={14} />
                Run ID
              </div>
              <div style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                {run.metadata.run_id}
              </div>
            </div>

            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "0.5rem",
                  color: "var(--text-muted)",
                  fontSize: "0.875rem",
                }}
              >
                <Video size={14} />
                Source Video
              </div>
              <div style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                {run.metadata.original_name}
              </div>
            </div>

            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "0.5rem",
                  color: "var(--text-muted)",
                  fontSize: "0.875rem",
                }}
              >
                <Calendar size={14} />
                Created At
              </div>
              <div style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                {new Date(run.metadata.created_at).toLocaleString()}
              </div>
            </div>
          </div>
        </div>

        <div className="glass-card">
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.5rem",
              marginBottom: "1.5rem",
              borderBottom: "1px solid var(--border-color)",
              paddingBottom: "0.75rem",
            }}
          >
            <Database size={18} color="var(--accent-secondary)" />
            <h3 style={{ fontSize: "1rem" }}>Artifacts</h3>
          </div>
          <div className="empty-state" style={{ padding: "2rem" }}>
            <p style={{ fontSize: "0.875rem" }}>
              No artifacts found for this run yet.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};

export default RunDetailsPage;
