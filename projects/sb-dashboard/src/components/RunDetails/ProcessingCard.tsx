import React from "react";
import { Play, AlertCircle, Square } from "lucide-react";
import type { RunDetail, ProcessingProgress } from "../../types/run";

interface ProcessingCardProps {
    run: RunDetail;
    isProcessing: boolean;
    processingProgress: ProcessingProgress | null;
    processingError: string | null;
    handleStartProcessing: () => void;
    handleStopProcessing: () => void;
}

const STAGE_ORDER = ["reader", "crop", "detect", "feature", "finalize"];

const ProcessingCard: React.FC<ProcessingCardProps> = ({
    run,
    isProcessing,
    processingProgress,
    processingError,
    handleStartProcessing,
    handleStopProcessing,
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
                <Play size={18} color="var(--accent-secondary)" />
                <h3 style={{ fontSize: "1rem" }}>Video Processing</h3>
            </div>

            {processingError && (
                <div
                    style={{
                        background: "rgba(239, 68, 68, 0.1)",
                        border: "1px solid rgba(239, 68, 68, 0.2)",
                        borderRadius: "8px",
                        padding: "0.75rem 1rem",
                        marginBottom: "1rem",
                        color: "#ef4444",
                        fontSize: "0.875rem",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.5rem",
                    }}
                >
                    <AlertCircle size={16} />
                    {processingError}
                </div>
            )}

            {processingProgress && (
                <div style={{ marginBottom: "1.5rem" }}>
                    {/* E2E Progress Bar */}
                    <div
                        style={{
                            display: "flex",
                            justifyContent: "space-between",
                            marginBottom: "0.5rem",
                            fontSize: "0.875rem",
                        }}
                    >
                        <span style={{ fontWeight: 500 }}>
                            {processingProgress.is_complete ? "Processing complete" : isProcessing ? "Processing..." : "Paused"}
                        </span>
                        <span>
                            {Math.round(((processingProgress.stages.finalize?.current ?? 0) / (processingProgress.total_frames || 1)) * 100)}%
                        </span>
                    </div>
                    <div
                        style={{
                            background: "var(--bg-secondary)",
                            borderRadius: "4px",
                            height: "10px",
                            overflow: "hidden",
                            marginBottom: "1rem",
                        }}
                    >
                        <div
                            style={{
                                background: "linear-gradient(90deg, #34d399, #06b6d4)",
                                height: "100%",
                                width: `${(processingProgress.total_frames > 0)
                                    ? ((processingProgress.stages.finalize?.current ?? 0) / processingProgress.total_frames) * 100
                                    : 0
                                    }%`,
                                transition: "width 0.3s ease",
                            }}
                        />
                    </div>

                    {/* Global Metrics - FPS based on the bottleneck (slowest) stage */}
                    {isProcessing && Object.values(processingProgress.stages).some(s => s.ms_per_frame > 0) && (
                        <div style={{
                            display: "flex",
                            justifyContent: "center",
                            background: "rgba(6, 182, 212, 0.05)",
                            padding: "0.5rem",
                            borderRadius: "6px",
                            fontSize: "0.875rem",
                            color: "var(--accent-secondary)",
                            fontWeight: 600,
                            marginBottom: "1rem"
                        }}>
                            Global: {(1000 / Math.max(...Object.values(processingProgress.stages).map(s => s.ms_per_frame || 0.1))).toFixed(1)} FPS
                        </div>
                    )}

                    {/* Stage Breakdown */}
                    <div style={{ display: "grid", gap: "0.75rem" }}>
                        {STAGE_ORDER.map((name) => {
                            const stage = processingProgress.stages[name];
                            if (!stage) return null;
                            return (
                                <div key={name} style={{
                                    fontSize: "0.75rem",
                                    display: "grid",
                                    gridTemplateColumns: "70px 100px 1fr 60px",
                                    alignItems: "center",
                                    gap: "1rem",
                                    padding: "0.25rem 0",
                                    borderBottom: "1px solid rgba(255,255,255,0.05)"
                                }}>
                                    <span style={{ textTransform: "capitalize", color: "var(--text-muted)", fontWeight: 500 }}>{name}</span>
                                    <span style={{ fontVariantNumeric: "tabular-nums", color: "var(--text-muted)" }}>
                                        {stage.current} / {stage.total}
                                    </span>
                                    <div style={{ height: "4px", background: "var(--bg-secondary)", borderRadius: "2px", overflow: "hidden" }}>
                                        <div style={{
                                            height: "100%",
                                            background: "var(--accent-secondary)",
                                            opacity: 0.5,
                                            width: `${(stage.total > 0) ? (stage.current / stage.total) * 100 : 0}%`,
                                            transition: "width 0.3s ease"
                                        }} />
                                    </div>
                                    <div style={{ textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                                        {stage.ms_per_frame > 0 ? `${stage.ms_per_frame.toFixed(1)}ms` : "--"}
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}

            {run.missing_dependencies.every((d) => d.valid) ? (
                isProcessing ? (
                    <button
                        onClick={handleStopProcessing}
                        className="btn btn-secondary"
                        style={{ width: "100%" }}
                    >
                        <Square size={18} />
                        Stop Processing
                    </button>
                ) : (
                    <button
                        onClick={handleStartProcessing}
                        className="btn btn-primary"
                        style={{ width: "100%" }}
                        disabled={processingProgress?.is_complete}
                    >
                        <Play size={18} />
                        {processingProgress?.is_complete ? "Processing Complete" : "Start Processing"}
                    </button>
                )
            ) : (
                <div
                    style={{
                        fontSize: "0.875rem",
                        color: "var(--text-muted)",
                        textAlign: "center",
                        padding: "1rem",
                        background: "var(--bg-secondary)",
                        borderRadius: "8px",
                    }}
                >
                    Resolve all dependencies above to enable processing.
                </div>
            )}
        </div>
    );
};

export default ProcessingCard;
