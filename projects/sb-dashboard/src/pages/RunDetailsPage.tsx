import React, { useEffect, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  Database,
  Info,
  Calendar,
  Video,
  Tag,
  Users,
  Edit2,
  Save,
  X,
  Loader2,
  Sun,
  Moon,
  Target,
  AlertCircle,
} from "lucide-react";
import BoundaryEditor from "../components/BoundaryEditor/BoundaryEditor";

interface RunDependency {
  artifact_name: string;
  message: string;
  valid: boolean;
}

interface RunDetail {
  run_id: string;
  metadata: {
    original_name: string;
    display_name: string;
    created_at: string;
    run_id: string;
    team_size: number;
    light_team_name: string;
    dark_team_name: string;
    tags: string[];
  };
  missing_dependencies: RunDependency[];
}

const RunDetailsPage: React.FC = () => {
  const { id } = useParams();
  const navigate = useNavigate();
  const [run, setRun] = useState<RunDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [showBoundaryEditor, setShowBoundaryEditor] = useState(false);

  // Edit State
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState("");
  const [editTeamSize, setEditTeamSize] = useState(7);
  const [editLightTeamName, setEditLightTeamName] = useState("");
  const [editDarkTeamName, setEditDarkTeamName] = useState("");
  const [editTags, setEditTags] = useState("");
  const [isSaving, setIsSaving] = useState(false);

  const fetchRun = React.useCallback(() => {
    setLoading(true);
    fetch(`/api/runs/${id}`)
      .then((res) => {
        if (!res.ok) throw new Error("Run not found");
        return res.json();
      })
      .then((data: RunDetail) => {
        setRun(data);
        setEditName(data.metadata.display_name);
        setEditTeamSize(data.metadata.team_size);
        setEditLightTeamName(data.metadata.light_team_name);
        setEditDarkTeamName(data.metadata.dark_team_name);
        setEditTags(data.metadata.tags.join(", "));
        setLoading(false);
      })
      .catch((err) => {
        console.error(err);
        setLoading(false);
      });
  }, [id]);

  useEffect(() => {
    fetchRun();
  }, [fetchRun]);

  const handleSave = async () => {
    if (!run) return;
    setIsSaving(true);

    const updatedMetadata = {
      ...run.metadata,
      display_name: editName,
      team_size: editTeamSize,
      light_team_name: editLightTeamName,
      dark_team_name: editDarkTeamName,
      tags: editTags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0),
    };

    try {
      const response = await fetch(`/api/runs/${run.run_id}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(updatedMetadata),
      });

      if (!response.ok) throw new Error("Failed to update run");

      const newMetadata = await response.json();
      setRun({ ...run, metadata: newMetadata });
      setIsEditing(false);
    } catch (err) {
      console.error(err);
      alert("Error saving changes");
    } finally {
      setIsSaving(false);
    }
  };

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
            {isEditing ? (
              <input
                className="form-input"
                style={{
                  fontSize: "1.5rem",
                  fontWeight: 700,
                  padding: "0.25rem 0.75rem",
                  height: "auto",
                  background: "transparent",
                }}
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
              />
            ) : (
              <h1>{run.metadata.display_name}</h1>
            )}
            <p style={{ color: "var(--text-secondary)" }}>{run.metadata.run_id}</p>
          </div>
        </div>
        <div style={{ display: "flex", gap: "0.75rem" }}>
          {isEditing ? (
            <>
              <button
                onClick={handleSave}
                disabled={isSaving}
                className="btn btn-primary"
              >
                {isSaving ? (
                  <Loader2 className="animate-spin" size={18} />
                ) : (
                  <Save size={18} />
                )}
                Save
              </button>
              <button onClick={() => setIsEditing(false)} className="btn btn-secondary">
                <X size={18} />
                Cancel
              </button>
            </>
          ) : (
            <button onClick={() => setIsEditing(true)} className="btn btn-secondary">
              <Edit2 size={18} />
              Edit Metadata
            </button>
          )}
        </div>
      </div>

      {showBoundaryEditor ? (
        <BoundaryEditor
          runId={id!}
          onComplete={() => {
            setShowBoundaryEditor(false);
            fetchRun();
          }}
          onCancel={() => setShowBoundaryEditor(false)}
        />
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
          <div className="lg:col-span-2 space-y-8">
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
                <h3 style={{ fontSize: "1rem" }}>Properties</h3>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-x-12 gap-y-6">
                <div className="list-item">
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
                  <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
                    {run.metadata.original_name}
                  </div>
                </div>

                <div className="list-item">
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "0.5rem",
                      color: "var(--text-muted)",
                      fontSize: "0.875rem",
                    }}
                  >
                    <Users size={14} />
                    Team Size
                  </div>
                  {isEditing ? (
                    <input
                      type="number"
                      className="form-input"
                      style={{ width: "80px", padding: "0.25rem 0.5rem" }}
                      value={editTeamSize}
                      onChange={(e) => setEditTeamSize(parseInt(e.target.value) || 0)}
                    />
                  ) : (
                    <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
                      {run.metadata.team_size} players
                    </div>
                  )}
                </div>

                <div className="list-item">
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "0.5rem",
                      color: "var(--text-muted)",
                      fontSize: "0.875rem",
                    }}
                  >
                    <Sun size={14} />
                    Light Team Name
                  </div>
                  {isEditing ? (
                    <input
                      type="text"
                      className="form-input"
                      style={{ borderRadius: "0.25rem", padding: "0.25rem 0.5rem" }}
                      value={editLightTeamName}
                      onChange={(e) => setEditLightTeamName(e.target.value)}
                    />
                  ) : (
                    <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
                      {run.metadata.light_team_name}
                    </div>
                  )}
                </div>

                <div className="list-item">
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "0.5rem",
                      color: "var(--text-muted)",
                      fontSize: "0.875rem",
                    }}
                  >
                    <Moon size={14} />
                    Dark Team Name
                  </div>
                  {isEditing ? (
                    <input
                      type="text"
                      className="form-input"
                      style={{ borderRadius: "0.25rem", padding: "0.25rem 0.5rem" }}
                      value={editDarkTeamName}
                      onChange={(e) => setEditDarkTeamName(e.target.value)}
                    />
                  ) : (
                    <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
                      {run.metadata.dark_team_name}
                    </div>
                  )}
                </div>

                <div className="list-item">
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
                    Tags
                  </div>
                  {isEditing ? (
                    <input
                      type="text"
                      className="form-input"
                      style={{ width: "200px", padding: "0.25rem 0.5rem" }}
                      value={editTags}
                      onChange={(e) => setEditTags(e.target.value)}
                      placeholder="tag1, tag2..."
                    />
                  ) : (
                    <div style={{ display: "flex", gap: "0.35rem", flexWrap: "wrap" }}>
                      {run.metadata.tags.length > 0 ? (
                        run.metadata.tags.map((t) => (
                          <span
                            key={t}
                            className="badge"
                            style={{ fontSize: "0.7rem", padding: "0.15rem 0.5rem" }}
                          >
                            {t}
                          </span>
                        ))
                      ) : (
                        <span style={{ color: "var(--text-muted)", fontSize: "0.875rem" }}>
                          None
                        </span>
                      )}
                    </div>
                  )}
                </div>

                <div className="list-item">
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
                  <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
                    {new Date(run.metadata.created_at).toLocaleString()}
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div className="space-y-8">
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
                <Target size={18} color="var(--accent-secondary)" />
                <h3 style={{ fontSize: "1rem" }}>Processing Dependencies</h3>
              </div>

              <div className="grid" style={{ gap: "1rem" }}>
                {run.missing_dependencies.map((dep) => (
                  <div
                    key={dep.artifact_name}
                    className="list-item"
                    style={{
                      background: dep.valid ? "rgba(52, 211, 153, 0.05)" : "rgba(239, 68, 68, 0.05)",
                      border: dep.valid ? "1px solid rgba(52, 211, 153, 0.1)" : "1px solid rgba(239, 68, 68, 0.1)",
                      borderRadius: "8px",
                      padding: "1rem",
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "0.75rem",
                        marginBottom: "0.25rem",
                      }}
                    >
                      {dep.valid ? (
                        <div style={{ color: "#34d399", display: "flex", alignItems: "center", gap: "0.75rem" }}>
                          <Database size={16} />
                          <span style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                            {dep.artifact_name}
                          </span>
                        </div>
                      ) : (
                        <div style={{ color: "#ef4444", display: "flex", alignItems: "center", gap: "0.75rem" }}>
                          <AlertCircle size={16} />
                          <span style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                            {dep.artifact_name}
                          </span>
                        </div>
                      )}
                    </div>
                    <p
                      style={{
                        fontSize: "0.75rem",
                        color: "var(--text-secondary)",
                        marginLeft: "1.75rem",
                        marginBottom: dep.artifact_name === 'field_boundaries.json' ? '0.5rem' : 0
                      }}
                    >
                      {dep.message}
                    </p>
                    {dep.artifact_name === 'field_boundaries.json' && (
                      <button
                        onClick={() => setShowBoundaryEditor(true)}
                        className="btn btn-secondary btn-sm"
                        style={{ marginLeft: '1.75rem', padding: '4px 8px', fontSize: '0.7rem' }}
                      >
                        <Target size={12} />
                        {dep.valid ? "Edit Boundaries" : "Define Boundaries"}
                      </button>
                    )}
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default RunDetailsPage;
