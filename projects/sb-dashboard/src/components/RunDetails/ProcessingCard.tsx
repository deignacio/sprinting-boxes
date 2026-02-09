import React from "react";
import { Play, AlertCircle, Database, Square } from "lucide-react";
import type { RunDetail, ProcessingProgress } from "../../types/run";

interface ProcessingCardProps {
    run: RunDetail;
    isProcessing: boolean;
    processingProgress: ProcessingProgress | null;
    processingError: string | null;
    handleStartProcessing: () => void;
    handleStopProcessing: () => void;
}

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
                <div style={{ marginBottom: "1rem" }}>
                    <div
                        style={{
                            display: "flex",
                            justifyContent: "space-between",
                            marginBottom: "0.5rem",
                            fontSize: "0.875rem",
                        }}
                    >
                        <span>{isProcessing ? "Processing frames..." : "Processing paused or complete"}</span>
                        <span>
                            {processingProgress.frames_processed} / {processingProgress.total_frames}
                        </span>
                    </div>
                    <div
                        style={{
                            background: "var(--bg-secondary)",
                            borderRadius: "4px",
                            height: "8px",
                            overflow: "hidden",
                        }}
                    >
                        <div
                            style={{
                                background: "linear-gradient(90deg, #34d399, #06b6d4)",
                                height: "100%",
                                width: `${processingProgress.total_frames > 0
                                    ? (processingProgress.frames_processed / processingProgress.total_frames) * 100
                                    : 0
                                    }%`,
                                transition: "width 0.3s ease",
                            }}
                        />
                    </div>
                    <div
                        style={{
                            fontSize: "0.75rem",
                            color: "var(--text-muted)",
                            marginTop: "0.5rem",
                        }}
                    >
                        Frames read: {processingProgress.frames_read}
                    </div>
                </div>
            )}

            {processingProgress?.is_complete && (
                <div
                    style={{
                        background: "rgba(52, 211, 153, 0.1)",
                        border: "1px solid rgba(52, 211, 153, 0.2)",
                        borderRadius: "8px",
                        padding: "0.75rem 1rem",
                        marginBottom: "1rem",
                        color: "#34d399",
                        fontSize: "0.875rem",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.5rem",
                    }}
                >
                    <Database size={16} />
                    Processing complete! {processingProgress.frames_processed} frames processed.
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
