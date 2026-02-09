import React from "react";
import { ArrowLeft, Database, Save, Loader2, X, Edit2 } from "lucide-react";
import type { RunDetail } from "../../types/run";

interface RunHeaderProps {
    run: RunDetail;
    isEditing: boolean;
    setIsEditing: (val: boolean) => void;
    editName: string;
    setEditName: (val: string) => void;
    isSaving: boolean;
    onSave: () => void;
    onBack: () => void;
}

const RunHeader: React.FC<RunHeaderProps> = ({
    run,
    isEditing,
    setIsEditing,
    editName,
    setEditName,
    isSaving,
    onSave,
    onBack,
}) => {
    return (
        <>
            <button
                onClick={onBack}
                className="btn btn-secondary"
                style={{ marginBottom: "2rem" }}
            >
                <ArrowLeft size={18} />
                Back to Dashboard
            </button>

            <div className="nav" style={{ border: "none", marginBottom: "1rem" }}>
                <div style={{ display: "flex", alignItems: "center", gap: "1rem" }}>
                    <div
                        style={{
                            width: "48px",
                            height: "48px",
                            background: "rgba(52, 211, 153, 0.1)",
                            borderRadius: "12px",
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "center",
                        }}
                    >
                        <Database size={24} color="#34d399" />
                    </div>
                    <div>
                        {isEditing ? (
                            <input
                                className="form-input"
                                style={{
                                    fontSize: "1.5rem",
                                    fontWeight: 700,
                                    padding: "0.25rem 0.75rem",
                                    height: "auto",
                                    background: "transparent",
                                }}
                                value={editName}
                                onChange={(e) => setEditName(e.target.value)}
                            />
                        ) : (
                            <h1>{run.run_context.display_name}</h1>
                        )}
                        <p style={{ color: "var(--text-secondary)" }}>{run.run_context.run_id}</p>
                    </div>
                </div>
                <div style={{ display: "flex", gap: "0.75rem" }}>
                    {isEditing ? (
                        <>
                            <button
                                onClick={onSave}
                                disabled={isSaving}
                                className="btn btn-primary"
                            >
                                {isSaving ? (
                                    <Loader2 className="animate-spin" size={18} />
                                ) : (
                                    <Save size={18} />
                                )}
                                Save
                            </button>
                            <button onClick={() => setIsEditing(false)} className="btn btn-secondary">
                                <X size={18} />
                                Cancel
                            </button>
                        </>
                    ) : (
                        <button onClick={() => setIsEditing(true)} className="btn btn-secondary">
                            <Edit2 size={18} />
                            Edit Metadata
                        </button>
                    )}
                </div>
            </div>
        </>
    );
};

export default RunHeader;
