import React from "react";
import { ArrowLeft, Database, Save, Loader2, X, Edit2 } from "lucide-react";
import type { RunDetail } from "../../types/run";

interface RunHeaderProps {
  run: RunDetail;
  isEditing: boolean;
  setIsEditing: (val: boolean) => void;
  editName: string;
  setEditName: (val: string) => void;
  isSaving: boolean;
  onSave: () => void;
  onBack: () => void;
}

const RunHeader: React.FC<RunHeaderProps> = ({
  run,
  isEditing,
  setIsEditing,
  editName,
  setEditName,
  isSaving,
  onSave,
  onBack,
}) => {
  return (
    <>
      <button
        onClick={onBack}
        className="btn btn-secondary"
        style={{ marginBottom: "0.5rem", padding: "0.5rem 1rem" }}
      >
        <ArrowLeft size={16} />
        Back
      </button>

      <div className="nav" style={{ border: "none", margin: "0", padding: "0.5rem 0" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "0.75rem" }}>
          <div
            style={{
              width: "36px",
              height: "36px",
              background: "rgba(52, 211, 153, 0.1)",
              borderRadius: "8px",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Database size={20} color="#34d399" />
          </div>
          <div>
            {isEditing ? (
              <input
                className="form-input"
                style={{
                  fontSize: "1.125rem",
                  fontWeight: 700,
                  padding: "0.25rem 0.5rem",
                  height: "auto",
                  background: "transparent",
                }}
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
              />
            ) : (
              <h1 style={{ fontSize: "1.125rem", lineHeight: 1.2 }}>{run.run_context.display_name}</h1>
            )}
            <p style={{ color: "var(--text-secondary)", fontSize: "0.75rem", lineHeight: 1.2 }}>
              {run.run_context.run_id}
            </p>
          </div>
        </div>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          {isEditing ? (
            <>
              <button
                onClick={onSave}
                disabled={isSaving}
                className="btn btn-primary"
                style={{ padding: "0.5rem 1rem" }}
              >
                {isSaving ? (
                  <Loader2 className="animate-spin" size={16} />
                ) : (
                  <Save size={16} />
                )}
                Save
              </button>
              <button
                onClick={() => setIsEditing(false)}
                className="btn btn-secondary"
                style={{ padding: "0.5rem 1rem" }}
              >
                <X size={16} />
                Cancel
              </button>
            </>
          ) : (
            <button
              onClick={() => setIsEditing(true)}
              className="btn btn-secondary"
              style={{ padding: "0.5rem 1rem" }}
            >
              <Edit2 size={16} />
              Edit
            </button>
          )}
        </div>
      </div>
    </>
  );
};

export default RunHeader;
