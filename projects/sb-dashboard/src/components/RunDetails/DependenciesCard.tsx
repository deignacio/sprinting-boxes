import React from "react";
import { Target, Database, AlertCircle } from "lucide-react";
import type { RunDetail } from "../../types/run";

interface DependenciesCardProps {
  run: RunDetail;
  setShowBoundaryEditor: (val: boolean) => void;
  onComputeCrops: () => void;
}

const DependenciesCard: React.FC<DependenciesCardProps> = ({
  run,
  setShowBoundaryEditor,
  onComputeCrops,
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
        <Target size={18} color="var(--accent-secondary)" />
        <h3 style={{ fontSize: "1rem" }}>Processing Dependencies</h3>
      </div>

      <div className="grid" style={{ gap: "1rem" }}>
        {run.missing_dependencies.map((dep) => (
          <div
            key={dep.artifact_name}
            className="list-item"
            style={{
              background: dep.valid
                ? "rgba(52, 211, 153, 0.05)"
                : "rgba(239, 68, 68, 0.05)",
              border: dep.valid
                ? "1px solid rgba(52, 211, 153, 0.1)"
                : "1px solid rgba(239, 68, 68, 0.1)",
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
                <div
                  style={{
                    color: "#34d399",
                    display: "flex",
                    alignItems: "center",
                    gap: "0.75rem",
                  }}
                >
                  <Database size={16} />
                  <span style={{ fontWeight: 600, fontSize: "0.875rem" }}>
                    {dep.artifact_name}
                  </span>
                </div>
              ) : (
                <div
                  style={{
                    color: "#ef4444",
                    display: "flex",
                    alignItems: "center",
                    gap: "0.75rem",
                  }}
                >
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
                marginBottom:
                  dep.artifact_name === "field_boundaries.json" ? "0.5rem" : 0,
              }}
            >
              {dep.message}
            </p>
            {dep.artifact_name === "field_boundaries.json" && (
              <button
                onClick={() => setShowBoundaryEditor(true)}
                className="btn btn-secondary btn-sm"
                style={{
                  marginLeft: "1.75rem",
                  padding: "4px 8px",
                  fontSize: "0.7rem",
                }}
              >
                <Target size={12} />
                {dep.valid ? "Edit Boundaries" : "Define Boundaries"}
              </button>
            )}
            {dep.artifact_name === "crops.json" && (
              <button
                onClick={onComputeCrops}
                className="btn btn-secondary btn-sm"
                disabled={
                  !run.missing_dependencies.find(
                    (d) => d.artifact_name === "field_boundaries.json",
                  )?.valid
                }
                style={{
                  marginLeft: "1.75rem",
                  padding: "4px 8px",
                  fontSize: "0.7rem",
                }}
              >
                <Database size={12} />
                {dep.valid ? "Re-compute Crops" : "Compute Crops"}
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};

export default DependenciesCard;
