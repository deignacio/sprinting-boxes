import React, { useRef, useState, useEffect, useMemo } from "react";
import { BOUNDARY_CHOICES } from "../../utils/boundaryUtils";
import type {
  Boundaries,
  BoundaryKey,
  Point,
  ROI,
} from "../../utils/boundaryUtils";
import type { EditorMode } from "./BoundaryControls";

interface BoundaryCanvasProps {
  imageUrl: string;
  boundaries: Boundaries;
  activeBoundary: BoundaryKey;
  mode: EditorMode;
  onBoundariesChange: (boundaries: Boundaries) => void;
  onImageLoad: (width: number, height: number) => void;
  roi: ROI | null;
}

const BoundaryCanvas: React.FC<BoundaryCanvasProps> = ({
  imageUrl,
  boundaries,
  activeBoundary,
  mode,
  onBoundariesChange,
  onImageLoad,
  roi,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [imageRes, setImageRes] = useState({ width: 0, height: 0 });
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState(false);
  const [panStart, setPanStart] = useState({ x: 0, y: 0 });
  const [dragPoint, setDragPoint] = useState<number | null>(null);

  useEffect(() => {
    const img = new Image();
    img.onload = () => {
      onImageLoad(img.width, img.height);
      setImageRes({ width: img.width, height: img.height });
    };
    img.src = imageUrl;
  }, [imageUrl, onImageLoad]);

  useEffect(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      const step = 250 / scale;
      if (e.key === "+" || e.key === "=") {
        setScale((s) => Math.min(s * 1.2, 10));
      } else if (e.key === "-" || e.key === "_") {
        setScale((s) => Math.max(s / 1.2, 0.5));
      } else if (e.key === "ArrowUp") {
        setOffset((prev) => ({ ...prev, y: prev.y - step }));
      } else if (e.key === "ArrowDown") {
        setOffset((prev) => ({ ...prev, y: prev.y + step }));
      } else if (e.key === "ArrowLeft") {
        setOffset((prev) => ({ ...prev, x: prev.x - step }));
      } else if (e.key === "ArrowRight") {
        setOffset((prev) => ({ ...prev, x: prev.x + step }));
      }
    };
    window.addEventListener("keydown", handleGlobalKeyDown);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown);
  }, [scale]);

  const effectiveViewBox = useMemo(() => {
    if (imageRes.width === 0) return { x: 0, y: 0, width: 100, height: 100 };

    const w = imageRes.width / scale;
    const h = imageRes.height / scale;
    const x = (imageRes.width - w) / 2 + offset.x;
    const y = (imageRes.height - h) / 2 + offset.y;

    return { x, y, width: w, height: h };
  }, [imageRes, scale, offset]);

  const handleMouseDown = (e: React.MouseEvent<SVGSVGElement>) => {
    const svg = e.currentTarget;
    const pt = svg.createSVGPoint();
    pt.x = e.clientX;
    pt.y = e.clientY;
    const cursorPt = pt.matrixTransform(svg.getScreenCTM()?.inverse());

    if (!cursorPt) return;

    if (e.shiftKey) {
      setIsPanning(true);
      setPanStart({ x: e.clientX, y: e.clientY });
      return;
    }

    if (mode === "add") {
      // Snapping logic: find the closest point from ANY boundary
      let finalPt = { x: cursorPt.x, y: cursorPt.y };
      const snapThreshold = 20 / scale;
      let closestPt: Point | null = null;
      let minDistance = snapThreshold;

      Object.values(boundaries)
        .flat()
        .forEach((p) => {
          const dist = Math.hypot(p.x - cursorPt.x, p.y - cursorPt.y);
          if (dist < minDistance) {
            minDistance = dist;
            closestPt = p;
          }
        });

      if (closestPt) {
        finalPt = closestPt;
      }

      const newPoints = [...boundaries[activeBoundary], finalPt];
      onBoundariesChange({ ...boundaries, [activeBoundary]: newPoints });
    } else if (mode === "move") {
      const hitIndex = boundaries[activeBoundary].findIndex(
        (p) => Math.hypot(p.x - cursorPt.x, p.y - cursorPt.y) < 20 / scale,
      );
      if (hitIndex !== -1) {
        setDragPoint(hitIndex);
      }
    } else if (mode === "delete") {
      const hitIndex = boundaries[activeBoundary].findIndex(
        (p) => Math.hypot(p.x - cursorPt.x, p.y - cursorPt.y) < 20 / scale,
      );
      if (hitIndex !== -1) {
        const newPoints = boundaries[activeBoundary].filter(
          (_, i) => i !== hitIndex,
        );
        onBoundariesChange({ ...boundaries, [activeBoundary]: newPoints });
      }
    }
  };

  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (isPanning) {
      const dx = ((e.clientX - panStart.x) * 5) / scale;
      const dy = ((e.clientY - panStart.y) * 5) / scale;
      setOffset((prev) => ({ x: prev.x - dx, y: prev.y - dy }));
      setPanStart({ x: e.clientX, y: e.clientY });
      return;
    }
    if (dragPoint !== null) {
      const svg = e.currentTarget;
      const pt = svg.createSVGPoint();
      pt.x = e.clientX;
      pt.y = e.clientY;
      const cursorPt = pt.matrixTransform(svg.getScreenCTM()?.inverse());

      if (!cursorPt) return;

      const newPoints = [...boundaries[activeBoundary]];
      newPoints[dragPoint] = { x: cursorPt.x, y: cursorPt.y };
      onBoundariesChange({ ...boundaries, [activeBoundary]: newPoints });
    }
  };

  const handleMouseUp = () => {
    setIsPanning(false);
    setDragPoint(null);
  };

  return (
    <div className="canvas-container" ref={containerRef}>
      <svg
        viewBox={`${effectiveViewBox.x} ${effectiveViewBox.y} ${effectiveViewBox.width} ${effectiveViewBox.height}`}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        style={{ cursor: mode === "add" ? "crosshair" : "default" }}
      >
        <image
          href={imageUrl}
          x="0"
          y="0"
          width={imageRes.width}
          height={imageRes.height}
        />

        {roi && (
          <rect
            x={roi.x}
            y={roi.y}
            width={roi.width}
            height={roi.height}
            fill="rgba(255, 255, 0, 0.05)"
            stroke="yellow"
            strokeDasharray="5,5"
            strokeWidth={1 / scale}
          />
        )}

        {Object.entries(boundaries).map(([key, points]) => {
          const choice = BOUNDARY_CHOICES.find((c) => c.key === key);
          if (!choice) return null;
          const isActive = key === activeBoundary;
          const color = choice.color;

          return (
            <g key={key} style={{ pointerEvents: isActive ? "auto" : "none" }}>
              {points.length > 2 && (
                <polygon
                  points={points.map((p: Point) => `${p.x},${p.y}`).join(" ")}
                  fill={isActive ? `${color}44` : `${color}11`}
                  stroke={isActive ? color : `${color}88`}
                  strokeWidth={(isActive ? 2 : 1) / scale}
                  strokeDasharray={isActive ? "none" : "5,5"}
                />
              )}
              {points.map((p: Point, i: number) => (
                <circle
                  key={i}
                  cx={p.x}
                  cy={p.y}
                  r={(isActive ? 6 : 4) / scale}
                  fill={isActive ? color : `${color}88`}
                  stroke="white"
                  strokeWidth={1 / scale}
                />
              ))}
            </g>
          );
        })}
      </svg>
      <div className="canvas-hint">Shift+Drag to Pan | +/- to Zoom</div>
    </div>
  );
};

export default BoundaryCanvas;
