import { useState, useEffect, useCallback } from "react";
import { type CliffData, type AuditSettings, type FeatureData } from "../../utils/auditUtils";

interface CliffContextPanelProps {
  runId: string;
  cliff: CliffData;
  settings: AuditSettings;
  isReadOnly?: boolean;
  onConfirm: () => void;
  onReject: () => void;
  onToggleSide: () => void;
  onSwitchColors: () => void;
}

const LOOKBACK = 3;
const LOOKAHEAD = 15;
const FRAME_WINDOW = LOOKBACK + LOOKAHEAD + 1;
const FRAME_DURATION_MS = 200;
const CLIFF_FRAME_IDX = LOOKBACK;

export default function CliffContextPanel({
  runId,
  cliff,
  settings,
  isReadOnly = false,
  onConfirm,
  onReject,
  onToggleSide,
  onSwitchColors,
}: CliffContextPanelProps) {
  const [features, setFeatures] = useState<FeatureData[]>([]);
  const [loadedCount, setLoadedCount] = useState(0);
  const [currentFrameIdx, setCurrentFrameIdx] = useState(CLIFF_FRAME_IDX);
  const [isPaused, setIsPaused] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const frameIndices = Array.from({ length: FRAME_WINDOW }, (_, i) => cliff.frame_index - LOOKBACK + i);

  const frameImages = frameIndices.map(
    (frameIdx) =>
      `/api/runs/${runId}/crops/frame_${String(frameIdx).padStart(6, "0")}_overview.jpg?annotate=true`
  );

  useEffect(() => {
    const fetchFeatures = async () => {
      try {
        const response = await fetch(`/api/runs/${runId}/audit/features`);
        if (response.ok) {
          const allFeatures: FeatureData[] = await response.json();
          const windowFeatures = allFeatures.filter(
            (f) => f.frame_index >= frameIndices[0] && f.frame_index <= frameIndices[frameIndices.length - 1]
          );
          setFeatures(windowFeatures);
          setIsLoading(false);
        }
      } catch (err) {
        console.error("Failed to fetch features", err);
        setIsLoading(false);
      }
    };
    fetchFeatures();
  }, [runId, frameIndices]);

  useEffect(() => {
    if (loadedCount < FRAME_WINDOW || isPaused) return;

    const timer = setInterval(() => {
      setCurrentFrameIdx((i) => (i + 1) % FRAME_WINDOW);
    }, FRAME_DURATION_MS);

    return () => clearInterval(timer);
  }, [loadedCount, isPaused]);

  const handleImageLoad = useCallback(() => {
    setLoadedCount((c) => c + 1);
  }, []);

  const currentFrameData = features.find((f) => f.frame_index === frameIndices[currentFrameIdx]);

  const pullingSide = cliff.manual_side_override || (cliff.left_emptied_first ? "left" : "right");
  const pullingTeam =
    pullingSide === "left" ? settings.light_team_name : settings.dark_team_name;

  const isReady = loadedCount === FRAME_WINDOW;

  return (
    <div style={{ padding: "16px", background: "#0f172a", borderTop: "1px solid #334155", display: "flex", gap: "16px", maxHeight: "calc(100vh - 200px)" }}>
      {/* Left column: animation */}
      <div style={{ flex: 5, display: "flex", flexDirection: "column", gap: "12px", minWidth: "0", height: "100%" }}>
        {/* Info bar */}
        <div style={{ display: "flex", gap: "16px", alignItems: "center", flexWrap: "wrap", flexShrink: 0 }}>
          <div style={{ color: "#cbd5e1", fontSize: "0.875rem" }}>
            <strong>Timestamp:</strong> {cliff.timestamp}
          </div>
          <div style={{ color: "#cbd5e1", fontSize: "0.875rem" }}>
            <strong>Pulling:</strong> {pullingTeam} ({pullingSide})
          </div>
          <div style={{ color: "#cbd5e1", fontSize: "0.875rem" }}>
            <strong>Score:</strong> Light {cliff.score_light} - Dark {cliff.score_dark}
          </div>
        </div>

        {/* Animation area */}
        <div
          style={{
            flex: 1,
            position: "relative",
            background: "#1e293b",
            borderRadius: "4px",
            overflow: "hidden",
            minHeight: "0",
          }}
        >
        {!isReady && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "#94a3b8",
              zIndex: 10,
            }}
          >
            {isLoading ? "Loading..." : `Loading frames: ${loadedCount}/${FRAME_WINDOW}`}
          </div>
        )}

        {/* Visible frame */}
        {isReady && (
          <>
            <img
              src={frameImages[currentFrameIdx]}
              alt={`Frame ${currentFrameIdx}`}
              onClick={() => setIsPaused(!isPaused)}
              style={{
                width: "100%",
                height: "100%",
                objectFit: "contain",
                display: "block",
                outline: currentFrameIdx === CLIFF_FRAME_IDX ? "3px solid #ef4444" : "none",
                cursor: "pointer",
              }}
            />
            {currentFrameIdx === CLIFF_FRAME_IDX && (
              <div
                style={{
                  position: "absolute",
                  top: "8px",
                  right: "8px",
                  background: "#ef4444",
                  color: "#fff",
                  padding: "4px 8px",
                  borderRadius: "4px",
                  fontSize: "0.75rem",
                  fontWeight: "bold",
                }}
              >
                CLIFF
              </div>
            )}

            {/* Backward arrow */}
            <button
              onClick={() => {
                setIsPaused(true);
                setCurrentFrameIdx((i) => (i - 1 + FRAME_WINDOW) % FRAME_WINDOW);
              }}
              style={{
                position: "absolute",
                left: "15%",
                top: "50%",
                transform: "translateY(-50%)",
                background: "rgba(0, 0, 0, 0.5)",
                color: "#fff",
                border: "none",
                borderRadius: "50%",
                width: "48px",
                height: "48px",
                fontSize: "24px",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                transition: "background 0.2s",
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = "rgba(0, 0, 0, 0.8)";
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = "rgba(0, 0, 0, 0.5)";
              }}
            >
              ◀
            </button>

            {/* Forward arrow */}
            <button
              onClick={() => {
                setIsPaused(true);
                setCurrentFrameIdx((i) => (i + 1) % FRAME_WINDOW);
              }}
              style={{
                position: "absolute",
                right: "15%",
                top: "50%",
                transform: "translateY(-50%)",
                background: "rgba(0, 0, 0, 0.5)",
                color: "#fff",
                border: "none",
                borderRadius: "50%",
                width: "48px",
                height: "48px",
                fontSize: "24px",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                transition: "background 0.2s",
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = "rgba(0, 0, 0, 0.8)";
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = "rgba(0, 0, 0, 0.5)";
              }}
            >
              ▶
            </button>
          </>
        )}

        {/* Hidden preload images */}
        {frameImages.map((url, idx) => (
          <img
            key={idx}
            src={url}
            alt={`Preload frame ${idx}`}
            style={{ display: "none" }}
            onLoad={handleImageLoad}
          />
        ))}
        </div>

        {/* Controls */}
        <div style={{ display: "flex", alignItems: "center", gap: "12px", flexShrink: 0 }}>
          <button
            onClick={() => setIsPaused(!isPaused)}
            disabled={!isReady}
            style={{
              padding: "4px 8px",
              background: isReady ? "#475569" : "#334155",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              cursor: isReady ? "pointer" : "not-allowed",
              fontSize: "0.75rem",
            }}
          >
            {isPaused ? "▶" : "⏸"}
          </button>
          <div style={{ color: "#94a3b8", fontSize: "0.875rem" }}>
            Frame {currentFrameIdx + 1}/{FRAME_WINDOW}
          </div>
          <div style={{ display: "flex", gap: "4px", flex: 1 }}>
            {frameIndices.map((_, idx) => (
              <div
                key={idx}
                style={{
                  flex: 1,
                  height: "4px",
                  background: idx === currentFrameIdx ? "#10b981" : "#334155",
                  borderRadius: "2px",
                  cursor: "pointer",
                }}
                onClick={() => setCurrentFrameIdx(idx)}
              />
            ))}
          </div>
        </div>

        {/* Per-frame stats */}
        {currentFrameData && (
          <div style={{ color: "#94a3b8", fontSize: "0.875rem", flexShrink: 0 }}>
            L: {currentFrameData.left_count.toFixed(2)} &nbsp; R:{" "}
            {currentFrameData.right_count.toFixed(2)} &nbsp; F:{" "}
            {currentFrameData.field_count.toFixed(2)} &nbsp; Score:{" "}
            {currentFrameData.pre_point_score.toFixed(3)}
          </div>
        )}
      </div>

      {/* Right column: actions */}
      {!isReadOnly && (
        <div style={{ display: "flex", flexDirection: "column", gap: "8px", justifyContent: "flex-start", flex: 1, minWidth: "120px", height: "100%", flexShrink: 0 }}>
          <button
            onClick={onConfirm}
            style={{
              padding: "8px 12px",
              background: "#10b981",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
              fontSize: "0.875rem",
              fontWeight: "bold",
            }}
          >
            ✓ Confirm
          </button>
          <button
            onClick={onReject}
            style={{
              padding: "8px 12px",
              background: "#ef4444",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
              fontSize: "0.875rem",
              fontWeight: "bold",
            }}
          >
            ✗ Reject
          </button>
          <button
            onClick={onToggleSide}
            style={{
              padding: "8px 12px",
              background: "#3b82f6",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
              fontSize: "0.875rem",
              fontWeight: "bold",
            }}
          >
            ⇄ Toggle
          </button>
          <button
            onClick={onSwitchColors}
            style={{
              padding: "8px 12px",
              background: "#a855f7",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
              fontSize: "0.875rem",
              fontWeight: "bold",
            }}
          >
            🎨 Colors
          </button>
        </div>
      )}
    </div>
  );
}
