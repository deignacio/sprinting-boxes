import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { Play, Folder, ChevronRight, Calendar, Users } from "lucide-react";

interface VideoInfo {
  name: string;
  path: string;
}

interface RunInfo {
  name: string;
  run_context: {
    original_name: string;
    display_name: string;
    created_at: string;
    run_id: string;
    team_size: number;
    light_team_name: string;
    dark_team_name: string;
    tags: string[];
  };
}

const ListPage: React.FC = () => {
  const [videos, setVideos] = useState<VideoInfo[]>([]);
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([
      fetch("/api/videos").then((res) => res.json()),
      fetch("/api/runs").then((res) => res.json()),
    ]).then(([videoData, runData]) => {
      setVideos(videoData);
      setRuns(runData);
      setLoading(false);
    });
  }, []);

  if (loading) {
    return (
      <div className="container empty-state">
        <p>Loading Sprinting Boxes...</p>
      </div>
    );
  }

  const runIds = new Set(runs.map((r) => r.run_context.run_id));
  const availableVideos = videos.filter((video) => {
    const stem = video.name.replace(/\.[^/.]+$/, "");
    return !runIds.has(stem);
  });

  return (
    <div className="container">
      <nav className="nav">
        <div className="logo">Sprinting Boxes</div>
        <div className="badge badge-video">Dashboard</div>
      </nav>

      <div className="grid grid-cols-2">
        <section>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.75rem",
              marginBottom: "1.5rem",
            }}
          >
            <Play size={24} color="var(--accent-secondary)" />
            <h2>Available Videos</h2>
          </div>
          <div className="grid">
            {availableVideos.length === 0 ? (
              <div className="glass-card empty-state">No new videos found.</div>
            ) : (
              availableVideos.map((video) => (
                <Link
                  key={video.path}
                  to={`/create-run?video=${encodeURIComponent(video.path)}`}
                >
                  <div className="glass-card list-item">
                    <div className="list-item-content">
                      <h3>{video.name}</h3>
                      <p>{video.path}</p>
                    </div>
                    <ChevronRight size={20} color="var(--text-muted)" />
                  </div>
                </Link>
              ))
            )}
          </div>
        </section>

        <section>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.75rem",
              marginBottom: "1.5rem",
            }}
          >
            <Folder size={24} color="#34d399" />
            <h2>Processed Runs</h2>
          </div>
          <div className="grid">
            {runs.length === 0 ? (
              <div className="glass-card empty-state">No runs found.</div>
            ) : (
              runs.map((run) => (
                <Link
                  key={run.run_context.run_id}
                  to={`/run-details/${run.run_context.run_id}`}
                >
                  <div className="glass-card list-item">
                    <div className="list-item-content">
                      <h3>{run.run_context.display_name}</h3>
                      <div
                        style={{
                          display: "flex",
                          flexDirection: "column",
                          gap: "0.5rem",
                          marginTop: "0.5rem",
                        }}
                      >
                        <div style={{ display: "flex", gap: "1rem" }}>
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "0.25rem",
                              fontSize: "0.75rem",
                              color: "var(--text-muted)",
                            }}
                          >
                            <Calendar size={12} />
                            {new Date(
                              run.run_context.created_at,
                            ).toLocaleDateString()}
                          </div>
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "0.25rem",
                              fontSize: "0.75rem",
                              color: "var(--text-muted)",
                            }}
                          >
                            <Users size={12} />
                            Team: {run.run_context.team_size}
                          </div>
                        </div>
                        {run.run_context.tags.length > 0 && (
                          <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
                            {run.run_context.tags.map(tag => (
                              <span key={tag} className="badge" style={{ fontSize: '0.65rem', padding: '0.1rem 0.5rem', background: 'rgba(255,255,255,0.05)' }}>
                                {tag}
                              </span>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                    <ChevronRight size={20} color="var(--text-muted)" />
                  </div>
                </Link>
              ))
            )}
          </div>
        </section>
      </div>
    </div>
  );
};

export default ListPage;
