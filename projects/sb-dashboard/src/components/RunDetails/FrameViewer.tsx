import React, { useState, useEffect, useMemo, useRef } from "react";
import { type CliffData, type AuditSettings } from "../../utils/auditUtils";

interface FeatureData {
    frame_index: number;
    left_count: number;
    right_count: number;
    field_count: number;
    pre_point_score: number;
}

interface EnrichedFrame extends FeatureData {
    score_light: number;
    score_dark: number;
    is_cliff: boolean;
    cliff_status?: string;
}

interface FrameViewerProps {
    runId: string;
    allCliffs: CliffData[];
    settings: AuditSettings;
}

const ITEM_HEIGHT = 500; // Fixed height for virtualization
const BUFFER_ITEMS = 5;

const FrameViewer: React.FC<FrameViewerProps> = ({
    runId,
    allCliffs,
    settings,
}) => {
    const [features, setFeatures] = useState<FeatureData[]>([]);
    const [loading, setLoading] = useState(true);
    const containerRef = useRef<HTMLDivElement>(null);
    const [scrollTop, setScrollTop] = useState(0);

    // Load features on mount
    useEffect(() => {
        const loadFeatures = async () => {
            try {
                const response = await fetch(`/api/runs/${runId}/audit/features`);
                if (response.ok) {
                    const data = await response.json();
                    setFeatures(data);
                }
            } catch (err) {
                console.error("Failed to load features", err);
            } finally {
                setLoading(false);
            }
        };
        loadFeatures();
    }, [runId]);

    // Enrich features with scores from cliffs
    const enrichedFrames = useMemo(() => {
        if (features.length === 0) return [];

        let cliffIdx = 0;
        // Sort cliffs just in case
        const sortedCliffs = [...allCliffs].sort(
            (a, b) => a.frame_index - b.frame_index,
        );

        // Initial scores
        let currentScoreLight = settings.initial_score_light;
        let currentScoreDark = settings.initial_score_dark;

        return features.map((frame) => {
            // While the next cliff is at or before this frame, update score
            // Note: cliffs store the score AT that point.
            // So if frame_index >= cliff.frame_index, we should use that cliff's score?
            // Or rather: The score applies to the segment FOLLOWING the cliff?
            // Usually "Point 5" means the score became X-Y at that point.
            // So all frames >= cliff.frame_index share that score until next cliff.

            while (
                cliffIdx < sortedCliffs.length &&
                sortedCliffs[cliffIdx].frame_index <= frame.frame_index
            ) {
                currentScoreLight = sortedCliffs[cliffIdx].score_light;
                currentScoreDark = sortedCliffs[cliffIdx].score_dark;
                cliffIdx++;
            }
            // If we advanced past the current frame, we want the LAST valid cliff score.
            // Actually, the loop advances cliffIdx until it points to a cliff > frame.
            // Wait, my logic above: "sortedCliffs[cliffIdx].frame_index <= frame.frame_index"
            // increments cliffIdx. So after loop, cliffIdx points to first cliff > frame.
            // So the "current" active cliff is cliffIdx - 1.

            // However, we just updated currentScoreLight inside the loop.
            // So if we processed cliffs <= frame, currentScore holds the score of the last one.
            // Which is correct.

            // Check if this specific frame IS a cliff
            // We look at cliffIdx - 1 because we just incremented past it
            const lastProcessedCliff =
                cliffIdx > 0 ? sortedCliffs[cliffIdx - 1] : null;
            const isCliff =
                lastProcessedCliff?.frame_index === frame.frame_index &&
                lastProcessedCliff.status !== "FalsePositive";

            return {
                ...frame,
                score_light: currentScoreLight,
                score_dark: currentScoreDark,
                is_cliff: isCliff || false,
                cliff_status:
                    lastProcessedCliff?.frame_index === frame.frame_index
                        ? lastProcessedCliff.status
                        : undefined,
            };
        }) as EnrichedFrame[];
    }, [features, allCliffs, settings]);

    // Virtualization Logic
    const totalItems = enrichedFrames.length;
    const totalHeight = totalItems * ITEM_HEIGHT;

    // Calculate visible range
    const visibleStartIndex = Math.floor(scrollTop / ITEM_HEIGHT);
    const visibleEndIndex = Math.min(
        totalItems,
        Math.floor((scrollTop + (containerRef.current?.clientHeight || 800)) / ITEM_HEIGHT) + 1
    );

    const renderStartIndex = Math.max(0, visibleStartIndex - BUFFER_ITEMS);
    const renderEndIndex = Math.min(totalItems, visibleEndIndex + BUFFER_ITEMS);

    const visibleFrames = enrichedFrames.slice(renderStartIndex, renderEndIndex);

    const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
        setScrollTop(e.currentTarget.scrollTop);
    };

    const scrollToFrame = (index: number) => {
        if (containerRef.current) {
            containerRef.current.scrollTop = index * ITEM_HEIGHT;
        }
    };

    const jumpToNextPoint = () => {
        // Find first cliff after current visible start
        // We use visibleStartIndex + 1 to ensure we move forward if we are exactly on a cliff
        const currentFrameIdx = enrichedFrames[visibleStartIndex]?.frame_index || 0;

        const nextCliff = allCliffs.find(c => c.frame_index > currentFrameIdx && c.status !== "FalsePositive");
        if (nextCliff) {
            // Find index in frames list
            const frameListIdx = features.findIndex(f => f.frame_index === nextCliff.frame_index);
            if (frameListIdx !== -1) {
                scrollToFrame(frameListIdx);
            }
        }
    };

    const jumpToPrevPoint = () => {
        const currentFrameIdx = enrichedFrames[visibleStartIndex]?.frame_index || 0;
        // Find last cliff before current
        // We search reverse or filter
        const prevCliff = [...allCliffs].reverse().find(c => c.frame_index < currentFrameIdx && c.status !== "FalsePositive");
        if (prevCliff) {
            const frameListIdx = features.findIndex(f => f.frame_index === prevCliff.frame_index);
            if (frameListIdx !== -1) {
                scrollToFrame(frameListIdx);
            }
        }
    };


    if (loading) return <div className="p-8 text-slate-400">Loading frames...</div>;
    if (features.length === 0) return <div className="p-8 text-slate-400">No frames found.</div>;

    return (
        <div style={{ height: "calc(100vh - 180px)", display: "flex", flexDirection: "column" }}>
            {/* Navigation Tools */}
            <div
                style={{
                    padding: "12px",
                    background: "#1e293b",
                    borderBottom: "1px solid #334155",
                    display: "flex",
                    gap: "12px",
                    alignItems: "center",
                    justifyContent: "space-between"
                }}
            >
                <div style={{ color: "#f1f5f9", fontWeight: "bold" }}>
                    Frame Viewer ({visibleStartIndex + 1} / {totalItems})
                </div>
                <div style={{ display: "flex", gap: "8px" }}>
                    <button
                        onClick={jumpToPrevPoint}
                        style={{
                            padding: "8px 16px",
                            background: "#334155",
                            color: "#f1f5f9",
                            border: "none",
                            borderRadius: "4px",
                            cursor: "pointer",
                        }}
                    >
                        ↑ Prev Point
                    </button>
                    <button
                        onClick={jumpToNextPoint}
                        style={{
                            padding: "8px 16px",
                            background: "#334155",
                            color: "#f1f5f9",
                            border: "none",
                            borderRadius: "4px",
                            cursor: "pointer",
                        }}
                    >
                        Next Point ↓
                    </button>
                </div>
            </div>

            {/* Virtualized List Container */}
            <div
                ref={containerRef}
                onScroll={handleScroll}
                style={{
                    flex: 1,
                    overflowY: "auto",
                    position: "relative",
                    background: "#0f172a",
                }}
            >
                <div style={{ height: totalHeight, position: "relative" }}>
                    {visibleFrames.map((frame, i) => {
                        const absoluteIndex = renderStartIndex + i;
                        const top = absoluteIndex * ITEM_HEIGHT;

                        return (
                            <div
                                key={frame.frame_index}
                                style={{
                                    position: "absolute",
                                    top,
                                    left: 0,
                                    right: 0,
                                    height: ITEM_HEIGHT,
                                    padding: "16px",
                                    boxSizing: "border-box",
                                    borderBottom: "1px solid #334155",
                                    background: frame.is_cliff ? "#172554" : "transparent"
                                }}
                            >
                                {/* Header info */}
                                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "8px", color: frame.is_cliff ? "#fff" : "#94a3b8" }}>
                                    <div className="flex gap-4">
                                        <span style={{ fontWeight: "bold", fontSize: "1.1rem" }}>Frame {frame.frame_index}</span>
                                        {frame.is_cliff && (
                                            <span className="bg-blue-600 text-white px-2 py-0.5 rounded text-sm">POINT DETECTED</span>
                                        )}
                                        {!frame.is_cliff && frame.cliff_status === "FalsePositive" && (
                                            <span className="bg-red-900 text-red-200 px-2 py-0.5 rounded text-sm">REJECTED</span>
                                        )}
                                    </div>
                                    <div style={{ display: "flex", gap: "16px", alignItems: "center" }}>
                                        <span>Score: {frame.score_light} - {frame.score_dark}</span>
                                    </div>
                                </div>

                                {/* Image Container */}
                                <div style={{ height: "400px", background: "#000", borderRadius: "8px", overflow: "hidden", position: "relative" }}>
                                    <img
                                        src={`/api/runs/${runId}/crops/frame_${String(frame.frame_index).padStart(6, "0")}_overview.jpg?annotate=true`}
                                        alt={`Frame ${frame.frame_index}`}
                                        loading="lazy"
                                        style={{ width: "100%", height: "100%", objectFit: "contain" }}
                                        onError={(e) => {
                                            (e.target as HTMLImageElement).style.display = 'none';
                                        }}
                                    />

                                    {/* Overlay Stats */}
                                    <div style={{
                                        position: "absolute",
                                        bottom: 0,
                                        left: 0,
                                        right: 0,
                                        background: "rgba(0,0,0,0.7)",
                                        color: "#fff",
                                        padding: "8px 12px",
                                        display: "flex",
                                        gap: "16px",
                                        fontFamily: "monospace"
                                    }}>
                                        <span>L: {frame.left_count.toFixed(2)}</span>
                                        <span>R: {frame.right_count.toFixed(2)}</span>
                                        <span>F: {frame.field_count.toFixed(2)}</span>
                                    </div>
                                </div>
                            </div>
                        );
                    })}
                </div>
            </div>
        </div>
    );
};

export default FrameViewer;
