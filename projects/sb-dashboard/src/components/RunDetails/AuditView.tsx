import { useState, useEffect, useRef, useCallback } from "react";
import {
  recalculateAudit,
  type CliffData,
  type AuditSettings,
} from "../../utils/auditUtils";

interface AuditViewProps {
  runId: string;
  onCliffClick: (cliff: CliffData) => void;
}

export default function AuditView({ runId, onCliffClick }: AuditViewProps) {
  const [cliffs, setCliffs] = useState<CliffData[]>([]);
  const [settings, setSettings] = useState<AuditSettings>({
    light_team_name: "Light",
    dark_team_name: "Dark",
    initial_score_light: 0,
    initial_score_dark: 0,
    video_start_time: "00:00:00",
  });
  const [localSettings, setLocalSettings] = useState(settings);
  const isInputFocused = useRef(false);

  const loadAuditData = useCallback(async () => {
    try {
      const response = await fetch(`/api/runs/${runId}/audit/cliffs`);
      if (response.ok) {
        const data = await response.json();
        setCliffs(data.cliffs);
        setSettings(data.settings);
      }
    } catch (err) {
      console.error("Failed to load audit data", err);
    }
  }, [runId]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadAuditData();
  }, [loadAuditData]);

  useEffect(() => {
    if (!isInputFocused.current) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setLocalSettings(settings);
    }
  }, [settings]);

  const syncAuditData = async (updatedCliffs: CliffData[]) => {
    try {
      await fetch(`/api/runs/${runId}/audit/cliffs`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cliffs: updatedCliffs, settings }),
      });
      setCliffs(updatedCliffs);
    } catch (err) {
      console.error("Failed to sync audit data", err);
    }
  };

  const updateSettings = async (newSettings: AuditSettings) => {
    try {
      await fetch(`/api/runs/${runId}/audit/settings`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(newSettings),
      });
      setSettings(newSettings);
      // Reload to get recalculated scores
      loadAuditData();
    } catch (err) {
      console.error("Failed to update settings", err);
    }
  };

  const handleAuditChange = (idx: number, field: string, value: string) => {
    const updated = cliffs.map((c, i) => {
      if (i === idx) return { ...c, [field]: value };
      return c;
    });

    const processed = recalculateAudit(updated, settings);
    syncAuditData(processed);
  };

  const handleStatusChange = (idx: number, newStatus: CliffData["status"]) => {
    const updated = cliffs.map((c, i) => {
      if (i === idx) return { ...c, status: newStatus };
      return c;
    });
    const processed = recalculateAudit(updated, settings);
    syncAuditData(processed);
  };

  if (cliffs.length === 0) {
    return (
      <div style={{ padding: "20px", textAlign: "center", color: "#94a3b8" }}>
        No point-start detections found. Run processing first.
      </div>
    );
  }

  return (
    <div style={{ padding: "20px" }}>
      <h2 style={{ marginBottom: "24px", color: "#f1f5f9" }}>
        Audit ({cliffs.filter((c) => c.status === "Confirmed").length}{" "}
        confirmed)
      </h2>

      <div
        style={{
          marginBottom: "24px",
          padding: "16px",
          background: "#1e293b",
          borderRadius: "8px",
          border: "1px solid #334155",
        }}
      >
        <h4
          style={{ margin: "0 0 16px 0", fontSize: "1rem", color: "#f1f5f9" }}
        >
          Global Audit Configuration
        </h4>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
            gap: "16px",
          }}
        >
          <div>
            <label
              style={{
                display: "block",
                fontSize: "0.75rem",
                color: "#94a3b8",
                marginBottom: "4px",
              }}
            >
              Light Team Name
            </label>
            <input
              type="text"
              value={localSettings.light_team_name}
              onChange={(e) =>
                setLocalSettings({
                  ...localSettings,
                  light_team_name: e.target.value,
                })
              }
              onFocus={() => {
                isInputFocused.current = true;
              }}
              onBlur={() => {
                isInputFocused.current = false;
                updateSettings(localSettings);
              }}
              onKeyDown={(e) =>
                e.key === "Enter" && updateSettings(localSettings)
              }
              style={{
                width: "100%",
                padding: "8px",
                background: "#0f172a",
                border: "1px solid #334155",
                borderRadius: "4px",
                color: "#f1f5f9",
              }}
            />
          </div>
          <div>
            <label
              style={{
                display: "block",
                fontSize: "0.75rem",
                color: "#94a3b8",
                marginBottom: "4px",
              }}
            >
              Dark Team Name
            </label>
            <input
              type="text"
              value={localSettings.dark_team_name}
              onChange={(e) =>
                setLocalSettings({
                  ...localSettings,
                  dark_team_name: e.target.value,
                })
              }
              onFocus={() => {
                isInputFocused.current = true;
              }}
              onBlur={() => {
                isInputFocused.current = false;
                updateSettings(localSettings);
              }}
              onKeyDown={(e) =>
                e.key === "Enter" && updateSettings(localSettings)
              }
              style={{
                width: "100%",
                padding: "8px",
                background: "#0f172a",
                border: "1px solid #334155",
                borderRadius: "4px",
                color: "#f1f5f9",
              }}
            />
          </div>
          <div>
            <label
              style={{
                display: "block",
                fontSize: "0.75rem",
                color: "#94a3b8",
                marginBottom: "4px",
              }}
            >
              Initial Score (L - D)
            </label>
            <div style={{ display: "flex", gap: "8px" }}>
              <input
                type="number"
                value={localSettings.initial_score_light}
                onChange={(e) =>
                  setLocalSettings({
                    ...localSettings,
                    initial_score_light: parseInt(e.target.value) || 0,
                  })
                }
                onBlur={() => updateSettings(localSettings)}
                onKeyDown={(e) =>
                  e.key === "Enter" && updateSettings(localSettings)
                }
                style={{
                  width: "60px",
                  padding: "8px",
                  background: "#0f172a",
                  border: "1px solid #334155",
                  borderRadius: "4px",
                  color: "#f1f5f9",
                }}
              />
              <input
                type="number"
                value={localSettings.initial_score_dark}
                onChange={(e) =>
                  setLocalSettings({
                    ...localSettings,
                    initial_score_dark: parseInt(e.target.value) || 0,
                  })
                }
                onBlur={() => updateSettings(localSettings)}
                onKeyDown={(e) =>
                  e.key === "Enter" && updateSettings(localSettings)
                }
                style={{
                  width: "60px",
                  padding: "8px",
                  background: "#0f172a",
                  border: "1px solid #334155",
                  borderRadius: "4px",
                  color: "#f1f5f9",
                }}
              />
            </div>
          </div>
          <div>
            <label
              style={{
                display: "block",
                fontSize: "0.75rem",
                color: "#94a3b8",
                marginBottom: "4px",
              }}
            >
              Video Start Time (HH:MM:SS)
            </label>
            <input
              type="text"
              placeholder="00:00:00"
              value={localSettings.video_start_time}
              onChange={(e) =>
                setLocalSettings({
                  ...localSettings,
                  video_start_time: e.target.value,
                })
              }
              onFocus={() => {
                isInputFocused.current = true;
              }}
              onBlur={() => {
                isInputFocused.current = false;
                updateSettings(localSettings);
              }}
              onKeyDown={(e) =>
                e.key === "Enter" && updateSettings(localSettings)
              }
              style={{
                width: "100%",
                padding: "8px",
                background: "#0f172a",
                border: "1px solid #334155",
                borderRadius: "4px",
                color: "#f1f5f9",
              }}
            />
          </div>
        </div>
      </div>

      <div
        style={{
          marginBottom: "24px",
          padding: "16px",
          background: "#1e293b",
          borderRadius: "8px",
          border: "1px solid #334155",
        }}
      >
        <h4
          style={{ margin: "0 0 16px 0", fontSize: "1rem", color: "#f1f5f9" }}
        >
          Exports
        </h4>
        <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
          <div style={{ display: "flex", gap: "12px", alignItems: "center" }}>
            <span
              style={{ color: "#94a3b8", width: "180px", fontSize: "0.85rem" }}
            >
              YouTube Chapters:
            </span>
            <button
              onClick={async () => {
                try {
                  const response = await fetch(
                    `/api/runs/${runId}/export/youtube`,
                  );
                  if (response.ok) {
                    const text = await response.text();
                    await navigator.clipboard.writeText(text);
                    alert("YouTube chapters copied to clipboard!");
                  } else {
                    alert("Failed to generate export.");
                  }
                } catch (err) {
                  console.error(err);
                  alert("Error exporting chapters.");
                }
              }}
              style={{
                padding: "8px 16px",
                background: "#3b82f6",
                color: "white",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              📋 Copy
            </button>
            <button
              onClick={async () => {
                window.open(`/api/runs/${runId}/export/youtube`, "_blank");
              }}
              style={{
                padding: "8px 16px",
                background: "#0f172a",
                color: "#94a3b8",
                border: "1px solid #334155",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              ⬇️ Download
            </button>
          </div>

          <div style={{ display: "flex", gap: "12px", alignItems: "center" }}>
            <span
              style={{ color: "#94a3b8", width: "180px", fontSize: "0.85rem" }}
            >
              Insta360 Studio Clips:
            </span>
            <button
              onClick={async () => {
                try {
                  const response = await fetch(
                    `/api/runs/${runId}/export/studio-clips`,
                  );
                  if (response.ok) {
                    const text = await response.text();
                    await navigator.clipboard.writeText(text);
                    alert("Studio Clips XML copied to clipboard!");
                  } else {
                    alert("Failed to generate export.");
                  }
                } catch (err) {
                  console.error(err);
                  alert("Error exporting clips.");
                }
              }}
              style={{
                padding: "8px 16px",
                background: "#8b5cf6",
                color: "white",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              📋 Copy XML
            </button>
            <button
              onClick={async () => {
                window.open(`/api/runs/${runId}/export/studio-clips`, "_blank");
              }}
              style={{
                padding: "8px 16px",
                background: "#0f172a",
                color: "#94a3b8",
                border: "1px solid #334155",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              ⬇️ Download XML
            </button>
          </div>

          <div style={{ display: "flex", gap: "12px", alignItems: "center" }}>
            <span
              style={{ color: "#94a3b8", width: "180px", fontSize: "0.85rem" }}
            >
              VLC Playlist (.m3u):
            </span>
            <button
              onClick={async () => {
                try {
                  const response = await fetch(
                    `/api/runs/${runId}/export/vlc-playlist`,
                  );
                  if (response.ok) {
                    const text = await response.text();
                    await navigator.clipboard.writeText(text);
                    alert("VLC Playlist copied to clipboard!");
                  } else {
                    alert("Failed to generate export.");
                  }
                } catch (err) {
                  console.error(err);
                  alert("Error exporting playlist.");
                }
              }}
              style={{
                padding: "8px 16px",
                background: "#f97316", // Orange-500
                color: "white",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              📋 Copy
            </button>
            <button
              onClick={async () => {
                try {
                  const response = await fetch(
                    `/api/runs/${runId}/export/vlc-playlist`,
                    { method: "POST" },
                  );
                  if (response.ok) {
                    alert("Playlist saved to run output directory!");
                  } else {
                    alert("Failed to save playlist.");
                  }
                } catch (err) {
                  console.error(err);
                  alert("Error saving playlist.");
                }
              }}
              style={{
                padding: "8px 16px",
                background: "#0f172a",
                color: "#94a3b8",
                border: "1px solid #334155",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              💾 Save to Disk
            </button>
          </div>

          <div
            style={{
              marginTop: "8px",
              paddingTop: "16px",
              borderTop: "1px solid #334155",
              display: "flex",
              gap: "12px",
              alignItems: "center",
            }}
          >
            <span
              style={{ color: "#94a3b8", width: "180px", fontSize: "0.85rem" }}
            >
              Troubleshooting:
            </span>
            <button
              onClick={async () => {
                try {
                  const response = await fetch(
                    `/api/runs/${runId}/metadata/backfill`,
                    { method: "POST" },
                  );
                  if (response.ok) {
                    alert("Metadata backfilled successfully!");
                  } else {
                    alert("Failed to backfill metadata.");
                  }
                } catch (err) {
                  console.error(err);
                  alert("Error backfilling metadata.");
                }
              }}
              style={{
                padding: "8px 16px",
                background: "#475569",
                color: "white",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "0.85rem",
              }}
            >
              ⚙️ Backfill Metadata
            </button>
          </div>
        </div>
      </div>

      <div
        style={{
          marginBottom: "16px",
          display: "flex",
          gap: "12px",
          color: "#94a3b8",
          fontSize: "0.85rem",
          background: "#1e293b",
          padding: "8px 16px",
          borderRadius: "4px",
          border: "1px solid #334155",
        }}
      >
        <span>
          Team A (Light): <strong>{settings.light_team_name}</strong>
        </span>
        <span style={{ color: "#475569" }}>|</span>
        <span>
          Team B (Dark): <strong>{settings.dark_team_name}</strong>
        </span>
      </div>

      <div style={{ overflowX: "auto" }}>
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr style={{ borderBottom: "1px solid #334155" }}>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                #
              </th>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                Timestamp
              </th>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                Side Assignment (L / R)
              </th>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                Pulling Side
              </th>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                Score ({settings.light_team_name} - {settings.dark_team_name})
              </th>
              <th
                style={{ padding: "12px", textAlign: "left", color: "#94a3b8" }}
              >
                Status / Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {cliffs.map((cliff, idx) => {
              const isFP = cliff.status === "FalsePositive";
              const isConfirmed = cliff.status === "Confirmed";

              let pullSide = "Unknown";
              let pullTeam = "";
              if (cliff.manual_side_override) {
                pullSide = cliff.manual_side_override.toUpperCase();
                const side = cliff.manual_side_override;
                const leftColor = cliff.left_team_color || "light";
                const pullingColor = (side === "left") ? leftColor : (leftColor === "light" ? "dark" : "light");
                pullTeam = pullingColor === "light" ? settings.light_team_name : settings.dark_team_name;
              } else if (cliff.left_emptied_first) {
                pullSide = "LEFT";
                pullTeam = (cliff.left_team_color || "light") === "light" ? settings.light_team_name : settings.dark_team_name;
              } else if (cliff.right_emptied_first) {
                pullSide = "RIGHT";
                const leftColor = cliff.left_team_color || "light";
                pullTeam = leftColor === "light" ? settings.dark_team_name : settings.light_team_name;
              }

              return (
                <tr
                  key={idx}
                  onClick={() => onCliffClick && onCliffClick(cliff)}
                  style={{
                    borderBottom: "1px solid #334155",
                    cursor: "pointer",
                    opacity: isFP ? 0.5 : 1,
                    background: isConfirmed ? "#1e40af20" : "transparent",
                  }}
                >
                  <td style={{ padding: "12px", color: "#f1f5f9" }}>
                    {idx + 1}
                  </td>
                  <td
                    style={{
                      padding: "12px",
                      color: "#f1f5f9",
                      position: "relative",
                    }}
                  >
                    {cliff.timestamp}
                    {cliff.is_break && (
                      <span
                        style={{
                          marginLeft: "8px",
                          padding: "2px 6px",
                          background: "#dc2626",
                          color: "white",
                          fontSize: "0.7rem",
                          borderRadius: "4px",
                        }}
                      >
                        🔥 BREAK
                      </span>
                    )}
                  </td>
                  <td
                    style={{ padding: "12px" }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    {isFP ? (
                      <span style={{ color: "#94a3b8", fontStyle: "italic" }}>
                        Skipped
                      </span>
                    ) : (
                      <select
                        value={cliff.left_team_color || "light"}
                        onChange={(e) =>
                          handleAuditChange(
                            idx,
                            "left_team_color",
                            e.target.value,
                          )
                        }
                        disabled={isConfirmed}
                        style={{
                          padding: "4px 8px",
                          background: "#0f172a",
                          border: "1px solid #334155",
                          borderRadius: "4px",
                          color: "#f1f5f9",
                        }}
                      >
                        <option value="light">
                          L:{settings.light_team_name} / R:
                          {settings.dark_team_name}
                        </option>
                        <option value="dark">
                          L:{settings.dark_team_name} / R:
                          {settings.light_team_name}
                        </option>
                      </select>
                    )}
                  </td>
                  <td style={{ padding: "12px", color: "#f1f5f9" }}>
                    <div style={{ fontWeight: "bold" }}>{pullSide}</div>
                    <div style={{ fontSize: "0.75rem", color: "#94a3b8" }}>{pullTeam}</div>
                  </td>
                  <td
                    style={{
                      padding: "12px",
                      color: "#f1f5f9",
                      fontWeight: isConfirmed ? "bold" : "normal",
                    }}
                  >
                    {cliff.score_light} - {cliff.score_dark}
                  </td>
                  <td
                    style={{ padding: "12px" }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    {isConfirmed ? (
                      <>
                        <span
                          style={{
                            padding: "4px 8px",
                            background: "#10b981",
                            color: "white",
                            fontSize: "0.75rem",
                            borderRadius: "4px",
                            marginRight: "8px",
                          }}
                        >
                          LOCKED
                        </span>
                        <button
                          onClick={() => handleStatusChange(idx, "Unconfirmed")}
                          style={{
                            padding: "4px 8px",
                            background: "transparent",
                            color: "#60a5fa",
                            border: "none",
                            cursor: "pointer",
                            textDecoration: "underline",
                          }}
                        >
                          Unlock
                        </button>
                      </>
                    ) : isFP ? (
                      <>
                        <span
                          style={{
                            padding: "4px 8px",
                            background: "#dc2626",
                            color: "white",
                            fontSize: "0.75rem",
                            borderRadius: "4px",
                            marginRight: "8px",
                          }}
                        >
                          REJECTED
                        </span>
                        <button
                          onClick={() => handleStatusChange(idx, "Unconfirmed")}
                          style={{
                            padding: "4px 8px",
                            background: "transparent",
                            color: "#60a5fa",
                            border: "none",
                            cursor: "pointer",
                            textDecoration: "underline",
                          }}
                        >
                          Restore
                        </button>
                      </>
                    ) : (
                      <div style={{ display: "flex", gap: "8px" }}>
                        <button
                          onClick={() => handleStatusChange(idx, "Confirmed")}
                          style={{
                            padding: "4px 12px",
                            background: "#10b981",
                            color: "white",
                            border: "none",
                            borderRadius: "4px",
                            cursor: "pointer",
                            fontSize: "0.75rem",
                          }}
                        >
                          Confirm
                        </button>
                        <button
                          onClick={() =>
                            handleStatusChange(idx, "FalsePositive")
                          }
                          style={{
                            padding: "4px 12px",
                            background: "transparent",
                            color: "#dc2626",
                            border: "1px solid #dc2626",
                            borderRadius: "4px",
                            cursor: "pointer",
                            fontSize: "0.75rem",
                          }}
                        >
                          Reject
                        </button>
                      </div>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
