import { useState } from "react";
import { useSearchParams, useNavigate } from "react-router-dom";
import { ArrowLeft, Play, Send, Loader2 } from "lucide-react";

const CreateRunPage: React.FC = () => {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const videoPath = searchParams.get("video") || "";
  const videoName = videoPath.split("/").pop() || "Unknown Video";

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleCreateRun = async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await fetch("/api/runs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ video_path: videoPath }),
      });

      if (!response.ok) {
        throw new Error("Failed to create run");
      }

      const metadata = await response.json();
      navigate(`/run-details/${metadata.run_id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
      setLoading(false);
    }
  };

  return (
    <div className="container">
      <button
        onClick={() => navigate(-1)}
        className="btn btn-secondary"
        style={{ marginBottom: "2rem" }}
      >
        <ArrowLeft size={18} />
        Back to List
      </button>

      <div
        className="glass-card"
        style={{ maxWidth: "600px", margin: "0 auto" }}
      >
        <div style={{ textAlign: "center", marginBottom: "2rem" }}>
          <div
            style={{
              width: "64px",
              height: "64px",
              background: "rgba(59, 130, 246, 0.1)",
              borderRadius: "50%",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              margin: "0 auto 1.5rem",
            }}
          >
            <Play size={32} color="var(--accent-secondary)" />
          </div>
          <h1>New Processing Run</h1>
          <p style={{ color: "var(--text-secondary)", marginTop: "0.5rem" }}>
            Initialize analysis for {videoName}
          </p>
        </div>

        <div style={{ marginBottom: "2rem" }}>
          <label
            style={{
              display: "block",
              fontSize: "0.75rem",
              fontWeight: 600,
              color: "var(--text-muted)",
              textTransform: "uppercase",
              marginBottom: "0.5rem",
            }}
          >
            Video Source
          </label>
          <div
            style={{
              padding: "1rem",
              background: "var(--bg-tertiary)",
              borderRadius: "0.5rem",
              fontSize: "0.875rem",
              wordBreak: "break-all",
            }}
          >
            {videoPath}
          </div>
        </div>

        {error && (
          <div
            style={{
              padding: "1rem",
              background: "rgba(239, 68, 68, 0.1)",
              border: "1px solid rgba(239, 68, 68, 0.2)",
              borderRadius: "0.5rem",
              color: "#f87171",
              marginBottom: "2rem",
              fontSize: "0.875rem",
            }}
          >
            {error}
          </div>
        )}

        <button
          onClick={handleCreateRun}
          disabled={loading}
          className="btn btn-primary"
          style={{ width: "100%", justifyContent: "center", padding: "1rem" }}
        >
          {loading ? (
            <Loader2 className="animate-spin" size={20} />
          ) : (
            <>
              <Send size={20} />
              Initialize Run
            </>
          )}
        </button>
      </div>
    </div>
  );
};

export default CreateRunPage;
