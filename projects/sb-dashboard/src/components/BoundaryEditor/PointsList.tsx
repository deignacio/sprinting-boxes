import React, { useState } from "react";
import type { Point } from "../../utils/boundaryUtils";
import { X, ChevronRight, ChevronLeft } from "lucide-react";

interface PointsListProps {
  points: Point[];
  onPointsChange: (newPoints: Point[]) => void;
}

const PointsList: React.FC<PointsListProps> = ({ points, onPointsChange }) => {
  const [isCollapsed, setIsCollapsed] = useState(true);

  const handleDelete = (index: number) => {
    const newPoints = points.filter((_, i) => i !== index);
    onPointsChange(newPoints);
  };

  if (isCollapsed) {
    return (
      <button
        className="points-toggle-btn collapsed"
        onClick={() => setIsCollapsed(false)}
        title="Show Points"
      >
        <ChevronLeft size={20} />
      </button>
    );
  }

  return (
    <div className="points-list side-panel-section">
      <div className="section-header">
        <h3>Points ({points.length})</h3>
        <button
          className="icon-btn"
          onClick={() => setIsCollapsed(true)}
          title="Hide Points"
        >
          <ChevronRight size={18} />
        </button>
      </div>
      <div className="points-scroll-area">
        {points.length === 0 ? (
          <p className="hint">No points added yet.</p>
        ) : (
          <ul>
            {points.map((p, i) => (
              <li key={i}>
                <span>
                  {i + 1}. ({Math.round(p.x)}, {Math.round(p.y)})
                </span>
                <button onClick={() => handleDelete(i)} className="delete-btn">
                  <X size={14} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};

export default PointsList;
