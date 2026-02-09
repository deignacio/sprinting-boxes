import React, { useEffect, useState } from "react";
import { useSearchParams, useNavigate } from "react-router-dom";
import { ArrowLeft, Loader2, Play } from "lucide-react";

const CreateRunPage: React.FC = () => {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const videoPath = searchParams.get("video") || "";
  const stem =
    videoPath
      .split("/")
      .pop()
      ?.replace(/\.[^/.]+$/, "") || "Unknown Video";

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const handleCreateRun = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await fetch("/api/runs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ video_path: videoPath }),
      });

      if (!response.ok) {
        throw new Error("Failed to create run context");
      }

      const metadata = await response.json();
      navigate(`/run-details/${metadata.run_id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
      setLoading(false);
    }
  }, [videoPath, navigate]);

  useEffect(() => {
    if (!videoPath) {
      setError("No video path provided");
      setLoading(false);
      return;
    }

    const timer = setTimeout(() => {
      handleCreateRun();
    }, 1500); // Small delay to show the "Initializing" state

    return () => clearTimeout(timer);
  }, [videoPath, handleCreateRun]);

  return (
    <div className="container">
      <button
        onClick={() => navigate("/")}
        className="btn btn-secondary"
        style={{ marginBottom: "2rem" }}
      >
        <ArrowLeft size={18} />
        Back to List
      </button>

      <div
        className="glass-card"
        style={{
          maxWidth: "500px",
          margin: "4rem auto",
          textAlign: "center",
          padding: "3rem 2rem",
        }}
      >
        <div
          style={{
            width: "80px",
            height: "80px",
            background: "rgba(59, 130, 246, 0.1)",
            borderRadius: "50%",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            margin: "0 auto 2rem",
          }}
        >
          {loading ? (
            <Loader2
              className="animate-spin"
              size={40}
              color="var(--accent-primary)"
            />
          ) : (
            <Play size={40} color="var(--accent-secondary)" />
          )}
        </div>

        {loading ? (
          <>
            <h1>Initializing Analysis</h1>
            <p style={{ color: "var(--text-secondary)", marginTop: "1rem" }}>
              Creating workspace for <strong>{stem}</strong>...
            </p>
          </>
        ) : error ? (
          <>
            <h1 style={{ color: "#f87171" }}>Initialization Failed</h1>
            <p
              style={{
                color: "var(--text-secondary)",
                marginTop: "1rem",
                marginBottom: "2rem",
              }}
            >
              {error}
            </p>
            <button
              onClick={handleCreateRun}
              className="btn btn-primary"
              style={{ margin: "0 auto" }}
            >
              Retry
            </button>
          </>
        ) : (
          <h1>Workspace Ready</h1>
        )}
      </div>
    </div>
  );
};

export default CreateRunPage;
