import React from "react";
import {
  Info,
  Video,
  Users,
  Tag,
  Calendar,
  Video as VideoIcon,
} from "lucide-react";
import type { RunDetail } from "../../types/run";

interface PropertiesCardProps {
  run: RunDetail;
  isEditing: boolean;
  editTeamSize: number;
  setEditTeamSize: (val: number) => void;
  editTags: string;
  setEditTags: (val: string) => void;
  editSampleRate: number;
  setEditSampleRate: (val: number) => void;
}

const PropertiesCard: React.FC<PropertiesCardProps> = ({
  run,
  isEditing,
  editTeamSize,
  setEditTeamSize,
  editTags,
  setEditTags,
  editSampleRate,
  setEditSampleRate,
}) => {
  return (
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
            {run.run_context.original_name}
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
              {run.run_context.team_size} players
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
              {run.run_context.tags.length > 0 ? (
                run.run_context.tags.map((t) => (
                  <span
                    key={t}
                    className="badge"
                    style={{ fontSize: "0.7rem", padding: "0.15rem 0.5rem" }}
                  >
                    {t}
                  </span>
                ))
              ) : (
                <span
                  style={{ color: "var(--text-muted)", fontSize: "0.875rem" }}
                >
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
            <VideoIcon size={14} />
            Capture Rate
          </div>
          {isEditing ? (
            <div
              style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
            >
              <input
                type="number"
                step="0.1"
                className="form-input"
                style={{ width: "80px", padding: "0.25rem 0.5rem" }}
                value={editSampleRate}
                onChange={(e) =>
                  setEditSampleRate(parseFloat(e.target.value) || 0)
                }
              />
              <span
                style={{ fontSize: "0.875rem", color: "var(--text-muted)" }}
              >
                FPS
              </span>
            </div>
          ) : (
            <div style={{ fontWeight: 500, fontSize: "0.875rem" }}>
              {run.run_context.sample_rate} FPS
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
            {new Date(run.run_context.created_at).toLocaleString()}
          </div>
        </div>
      </div>
    </div>
  );
};

export default PropertiesCard;
