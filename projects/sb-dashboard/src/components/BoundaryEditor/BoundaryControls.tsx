import React from "react";
import { BOUNDARY_CHOICES } from "../../utils/boundaryUtils";
import type { BoundaryKey } from "../../utils/boundaryUtils";

export type EditorMode = "explore" | "add" | "move" | "delete";

interface BoundaryControlsProps {
  frames: string[];
  currentIndex: number;
  onFrameChange: (index: number) => void;
  activeBoundary: BoundaryKey;
  onBoundaryChange: (key: BoundaryKey) => void;
  mode: EditorMode;
  onModeChange: (mode: EditorMode) => void;
}

const BoundaryControls: React.FC<BoundaryControlsProps> = ({
  frames,
  currentIndex,
  onFrameChange,
  activeBoundary,
  onBoundaryChange,
  mode,
  onModeChange,
}) => {
  const modes: { key: EditorMode; label: string }[] = [
    { key: "explore", label: "Explore" },
    { key: "add", label: "Add Point" },
    { key: "move", label: "Move Point" },
    { key: "delete", label: "Delete Point" },
  ];

  return (
    <div className="editor-controls">
      <div className="control-group">
        <label>Reference Frame:</label>
        <select
          value={currentIndex}
          onChange={(e) => onFrameChange(Number(e.target.value))}
        >
          {frames.map((f, i) => (
            <option key={i} value={i}>
              Frame {i + 1} ({f})
            </option>
          ))}
        </select>
      </div>

      <div className="control-group">
        <label>Zone:</label>
        <div className="toggle-group">
          {BOUNDARY_CHOICES.map((c) => (
            <button
              key={c.key}
              className={activeBoundary === c.key ? "active" : ""}
              onClick={() => onBoundaryChange(c.key)}
              style={{ borderBottom: `3px solid ${c.color}` }}
            >
              {c.label}
            </button>
          ))}
        </div>
      </div>

      <div className="control-group">
        <label>Mode:</label>
        <div className="toggle-group">
          {modes.map((m) => (
            <button
              key={m.key}
              className={mode === m.key ? "active" : ""}
              onClick={() => onModeChange(m.key)}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
};

export default BoundaryControls;
