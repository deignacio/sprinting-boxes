import { useState, useEffect, useRef, useCallback } from "react";
import { type CliffData, type AuditSettings } from "../../utils/auditUtils";

interface CliffDetailProps {
  runId: string;
  cliff: CliffData;
  allCliffs: CliffData[];
  settings: AuditSettings;
  onUpdateSettings: (settings: AuditSettings) => Promise<void>;
  onBack: () => void;
  onNavigate: (cliff: CliffData) => void;
}

interface FrameData {
  frame_index: number;
  left_count: number;
  right_count: number;
  // field_count is available in data but used for score calc, not displayed yet
  pre_point_score: number;
  crop_path?: string;
}

export default function CliffDetail({
  runId,
  cliff,
  allCliffs,
  settings,
  onUpdateSettings,
  onBack,
  onNavigate,
}: CliffDetailProps) {
  const [localStatus, setLocalStatus] = useState(cliff.status);
  const [localSideOverride, setLocalSideOverride] = useState<string | null>(
    cliff.manual_side_override || null,
  );
  const [localLeftColor, setLocalLeftColor] = useState<string | null>(
    cliff.left_team_color || null,
  );
  const [localRightColor, setLocalRightColor] = useState<string | null>(
    cliff.right_team_color || null,
  );
  const [frames, setFrames] = useState<FrameData[]>([]);
  const [enlarged, setEnlarged] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Local settings state for inputs
  const [localSettings, setLocalSettings] = useState(settings);
  const isInputFocused = useRef(false);

  // Asymmetric lookback/lookahead as requested
  const lookback = 3;
  const lookahead = 15;

  const loadFrameData = useCallback(async () => {
    try {
      const response = await fetch(`/api/runs/${runId}/audit/features`);
      if (response.ok) {
        const data: FrameData[] = await response.json();
        const rangeStart = cliff.frame_index - lookback;
        const rangeEnd = cliff.frame_index + lookahead;
        const filtered = data.filter(
          (f) => f.frame_index >= rangeStart && f.frame_index <= rangeEnd,
        );

        const enriched = filtered.map((f) => ({
          ...f,
          crop_path: `/runs/${runId}/crops/crop_${runId}_${f.frame_index}.jpg`,
        }));

        setFrames(enriched);
      }
    } catch (err) {
      console.error("Failed to load frame data", err);
    }
  }, [runId, cliff.frame_index, lookback, lookahead]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadFrameData();
  }, [loadFrameData]);

  const currentIndex = allCliffs.findIndex(
    (c) => c.frame_index === cliff.frame_index,
  );
  const prevCliff = currentIndex > 0 ? allCliffs[currentIndex - 1] : null;
  const nextCliff =
    currentIndex < allCliffs.length - 1 ? allCliffs[currentIndex + 1] : null;

  // --- Dynamic Score Calculation ---
  // Start with scores from previous point (or initial scores if first)
  const prevScoreLight = prevCliff
    ? prevCliff.score_light
    : settings.initial_score_light;
  const prevScoreDark = prevCliff
    ? prevCliff.score_dark
    : settings.initial_score_dark;

  // Determine current point's contribution
  const currentPullSide =
    localSideOverride || (cliff.left_emptied_first ? "left" : "right");
  const currentLeftColor = localLeftColor || "light";
  const currentRightColor = localRightColor || "dark";
  const currentPullTeamColor =
    currentPullSide === "left" ? currentLeftColor : currentRightColor;

  // If status is FalsePositive, no score added
  const pointAddedLight =
    localStatus !== "FalsePositive" && currentPullTeamColor === "light" ? 1 : 0;
  const pointAddedDark =
    localStatus !== "FalsePositive" && currentPullTeamColor === "dark" ? 1 : 0;

  const currentScoreLight = prevScoreLight + pointAddedLight;
  const currentScoreDark = prevScoreDark + pointAddedDark;
  // ---------------------------------

  const handleConfirm = async () => {
    try {
      await fetch(
        `/api/runs/${runId}/audit/cliffs/${cliff.frame_index}/confirm`,
        { method: "POST" },
      );
      setLocalStatus("Confirmed");
    } catch (err) {
      console.error("Failed to confirm", err);
    }
  };

  const handleReject = async () => {
    try {
      await fetch(
        `/api/runs/${runId}/audit/cliffs/${cliff.frame_index}/reject`,
        { method: "POST" },
      );
      setLocalStatus("FalsePositive");
    } catch (err) {
      console.error("Failed to reject", err);
    }
  };

  const handleSwitchSide = async () => {
    try {
      await fetch(`/api/runs/${runId}/audit/cliffs/${cliff.frame_index}/side`, {
        method: "POST",
      });
      // Toggle local override
      const current = currentPullSide;
      setLocalSideOverride(current === "left" ? "right" : "left");
    } catch (err) {
      console.error("Failed to switch side", err);
    }
  };

  const handleSwitchColors = async () => {
    try {
      await fetch(
        `/api/runs/${runId}/audit/cliffs/${cliff.frame_index}/colors`,
        { method: "POST" },
      );

      // Toggle local colors
      const currentLeft = localLeftColor || "light";
      const newLeft = currentLeft === "light" ? "dark" : "light";
      const newRight = currentLeft === "light" ? "light" : "dark";

      setLocalLeftColor(newLeft);
      setLocalRightColor(newRight);
    } catch (err) {
      console.error("Failed to switch colors", err);
    }
  };

  const getTeamName = (side: "left" | "right") => {
    // Default assignment if not explicit
    const leftColor = localLeftColor || "light";
    const rightColor = localRightColor || "dark";

    const color = side === "left" ? leftColor : rightColor;
    return color === "light"
      ? settings.light_team_name
      : settings.dark_team_name;
  };

  return (
    <div style={{ padding: "20px", background: "#0f172a", minHeight: "100vh" }}>
      {/* Header */}
      <div
        style={{
          position: "sticky",
          top: 0,
          background: "#1e293b",
          padding: "16px",
          borderRadius: "8px",
          marginBottom: "24px",
          border: "1px solid #334155",
          zIndex: 10,
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            marginBottom: "16px",
          }}
        >
          <div style={{ display: "flex", gap: "12px", alignItems: "center" }}>
            <button
              onClick={onBack}
              style={{
                padding: "8px 16px",
                background: "#334155",
                color: "#f1f5f9",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
              }}
            >
              ← Back to Table
            </button>
            <button
              onClick={() => setShowSettings(!showSettings)}
              style={{
                padding: "8px 16px",
                background: "transparent",
                color: "#94a3b8",
                border: "1px solid #475569",
                borderRadius: "4px",
                cursor: "pointer",
              }}
            >
              ⚙️ Settings
            </button>
          </div>

          <h2 style={{ margin: 0, color: "#f1f5f9" }}>
            Frame {cliff.frame_index} - Score: {currentScoreLight} -{" "}
            {currentScoreDark}
            {cliff.is_break && (
              <span
                style={{
                  marginLeft: "12px",
                  padding: "4px 8px",
                  background: "#dc2626",
                  color: "white",
                  fontSize: "0.9rem",
                  borderRadius: "4px",
                }}
              >
                🔥 BREAK
              </span>
            )}
          </h2>
          <div style={{ display: "flex", gap: "8px" }}>
            <button
              onClick={() => prevCliff && onNavigate(prevCliff)}
              disabled={!prevCliff}
              style={{
                padding: "8px 16px",
                background: prevCliff ? "#334155" : "#1e293b",
                color: prevCliff ? "#f1f5f9" : "#64748b",
                border: "none",
                borderRadius: "4px",
                cursor: prevCliff ? "pointer" : "not-allowed",
              }}
            >
              ← Prev
            </button>
            <span style={{ padding: "8px 16px", color: "#94a3b8" }}>
              Point {currentIndex + 1} / {allCliffs.length}
            </span>
            <button
              onClick={() => nextCliff && onNavigate(nextCliff)}
              disabled={!nextCliff}
              style={{
                padding: "8px 16px",
                background: nextCliff ? "#334155" : "#1e293b",
                color: nextCliff ? "#f1f5f9" : "#64748b",
                border: "none",
                borderRadius: "4px",
                cursor: nextCliff ? "pointer" : "not-allowed",
              }}
            >
              Next →
            </button>
          </div>
        </div>

        {/* Settings Panel */}
        {showSettings && (
          <div
            style={{
              marginBottom: "16px",
              padding: "16px",
              background: "#0f172a",
              borderRadius: "8px",
              border: "1px solid #334155",
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
                  onUpdateSettings(localSettings);
                }}
                onKeyDown={(e) =>
                  e.key === "Enter" && onUpdateSettings(localSettings)
                }
                style={{
                  width: "100%",
                  padding: "8px",
                  background: "#1e293b",
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
                  onUpdateSettings(localSettings);
                }}
                onKeyDown={(e) =>
                  e.key === "Enter" && onUpdateSettings(localSettings)
                }
                style={{
                  width: "100%",
                  padding: "8px",
                  background: "#1e293b",
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
                  onBlur={() => onUpdateSettings(localSettings)}
                  style={{
                    width: "60px",
                    padding: "8px",
                    background: "#1e293b",
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
                  onBlur={() => onUpdateSettings(localSettings)}
                  style={{
                    width: "60px",
                    padding: "8px",
                    background: "#1e293b",
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
                Time Offset (sec)
              </label>
              <input
                type="number"
                step="0.1"
                value={localSettings.time_offset_secs}
                onChange={(e) =>
                  setLocalSettings({
                    ...localSettings,
                    time_offset_secs: parseFloat(e.target.value) || 0,
                  })
                }
                onBlur={() => onUpdateSettings(localSettings)}
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
                Video Start Time
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
                  onUpdateSettings(localSettings);
                }}
                onKeyDown={(e) =>
                  e.key === "Enter" && onUpdateSettings(localSettings)
                }
                style={{
                  width: "100%",
                  padding: "8px",
                  background: "#1e293b",
                  border: "1px solid #334155",
                  borderRadius: "4px",
                  color: "#f1f5f9",
                }}
              />
            </div>
          </div>
        )}

        {/* Action Buttons */}
        <div style={{ display: "flex", gap: "12px", flexWrap: "wrap" }}>
          <div
            style={{
              padding: "8px 16px",
              background:
                localStatus === "Confirmed"
                  ? "#10b981"
                  : localStatus === "FalsePositive"
                    ? "#dc2626"
                    : "#64748b",
              color: "white",
              borderRadius: "4px",
              fontWeight: "bold",
            }}
          >
            Status: {localStatus}
          </div>
          <button
            onClick={handleConfirm}
            disabled={localStatus === "Confirmed"}
            style={{
              padding: "8px 16px",
              background: localStatus === "Confirmed" ? "#1e293b" : "#10b981",
              color: "white",
              border: "none",
              borderRadius: "4px",
              cursor: localStatus === "Confirmed" ? "not-allowed" : "pointer",
            }}
          >
            ✓ Confirm
          </button>
          <button
            onClick={handleReject}
            disabled={localStatus === "FalsePositive"}
            style={{
              padding: "8px 16px",
              background:
                localStatus === "FalsePositive" ? "#1e293b" : "#dc2626",
              color: "white",
              border: "none",
              borderRadius: "4px",
              cursor:
                localStatus === "FalsePositive" ? "not-allowed" : "pointer",
            }}
          >
            ✗ Reject
          </button>
          <button
            onClick={handleSwitchSide}
            style={{
              padding: "8px 16px",
              background: "#3b82f6",
              color: "white",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
            }}
          >
            ⇄ Toggle Side (Current: {currentPullSide.toUpperCase()})
          </button>
          <button
            onClick={handleSwitchColors}
            style={{
              padding: "8px 16px",
              background: "#8b5cf6",
              color: "white",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
            }}
          >
            🎨 Switch Colors
          </button>
          <button
            onClick={() => setEnlarged(!enlarged)}
            style={{
              padding: "8px 16px",
              background: "#64748b",
              color: "white",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
            }}
          >
            🔍 {enlarged ? "Shrink Crops" : "Enlarge Crops"}
          </button>
        </div>
      </div>

      {/* Frame Sequence */}
      <div>
        <h3 style={{ color: "#f1f5f9", marginBottom: "16px" }}>
          Frame Sequence (-{lookback} / +{lookahead} frames)
        </h3>
        <div style={{ display: "flex", flexDirection: "column", gap: "24px" }}>
          {frames.map((frame, idx) => {
            const isCliffFrame = frame.frame_index === cliff.frame_index;
            const highlightSide = isCliffFrame ? currentPullSide : null;

            return (
              <div
                key={idx}
                style={{
                  padding: "16px",
                  background: isCliffFrame ? "#172554" : "#1e293b",
                  border: isCliffFrame
                    ? "2px solid #3b82f6"
                    : "1px solid #334155",
                  borderRadius: "8px",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    marginBottom: "12px",
                    color: "#f1f5f9",
                    fontWeight: isCliffFrame ? "bold" : "normal",
                  }}
                >
                  <span>
                    Frame {frame.frame_index}, Pre-Point Score:{" "}
                    {frame.pre_point_score.toFixed(3)}
                  </span>
                  {isCliffFrame && (
                    <span
                      style={{
                        background: "#3b82f6",
                        color: "white",
                        padding: "2px 8px",
                        borderRadius: "4px",
                        fontSize: "0.8rem",
                      }}
                    >
                      CLIFF Detected
                    </span>
                  )}
                </div>

                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: enlarged ? "1fr" : "1fr 1fr",
                    gap: "16px",
                  }}
                >
                  {["left", "right"].map((boundary) => {
                    const isHighlighted = highlightSide === boundary;
                    const count =
                      boundary === "left"
                        ? frame.left_count
                        : frame.right_count;
                    const teamName = getTeamName(boundary as "left" | "right");

                    // "align to the right on the left crop"
                    const bannerStyle: React.CSSProperties = {
                      position: "absolute",
                      bottom: 0,
                      background: "rgba(0,0,0,0.8)",
                      color: "white",
                      padding: "4px 8px",
                      fontSize: "0.85rem",
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                      borderTopLeftRadius: boundary === "right" ? "0" : "4px",
                      borderTopRightRadius: boundary === "left" ? "0" : "4px",
                    };

                    if (boundary === "left") {
                      bannerStyle.right = 0;
                      bannerStyle.textAlign = "right";
                    } else {
                      bannerStyle.left = 0;
                      bannerStyle.textAlign = "left";
                    }

                    return (
                      <div
                        key={boundary}
                        style={{
                          position: "relative",
                          border: isHighlighted
                            ? "3px solid #f59e0b"
                            : "1px solid #475569",
                          borderRadius: "4px",
                          overflow: "hidden",
                          cursor: "pointer",
                        }}
                        onClick={() => setEnlarged(!enlarged)}
                      >
                        <img
                          src={`/api/runs/${runId}/crops/frame_${String(frame.frame_index).padStart(6, "0")}_${boundary}.jpg`}
                          alt={`${boundary} crop`}
                          style={{
                            width: "100%",
                            display: "block",
                            minHeight: "200px",
                            objectFit: "contain",
                            background: "#000",
                          }}
                          loading="lazy"
                          onError={(e) => {
                            (e.target as HTMLImageElement).src =
                              "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMDAgMTAwIiBmaWxsPSIjMzMzIj48cmVjdCB4PSIwIiB5PSIwIiB3aWR0aD0iMTAwIiBoZWlnaHQ9IjEwMCIvPjx0ZXh0IHg9IjUwIiB5PSI1MCIgZmlsbD0iI2ZmZiIgdGV4dC1hbmNob3I9Im1pZGRsZSI+Tm8gSW1hZ2U8L3RleHQ+PC9zdmc+";
                          }}
                        />
                        <div style={bannerStyle}>
                          <span style={{ fontWeight: "bold" }}>
                            {teamName} ({boundary.toUpperCase()})
                          </span>
                          <span>| {count.toFixed(2)}</span>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
        {frames.length === 0 && (
          <div
            style={{ padding: "40px", textAlign: "center", color: "#64748b" }}
          >
            No frame data available. Ensure features have been computed.
          </div>
        )}
      </div>
    </div>
  );
}
