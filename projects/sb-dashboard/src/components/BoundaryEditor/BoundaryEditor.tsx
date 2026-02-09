import React, { useState, useEffect, useMemo } from 'react';
import BoundaryCanvas from './BoundaryCanvas';
import BoundaryControls from './BoundaryControls';
import type { EditorMode } from './BoundaryControls';
import PointsList from './PointsList';
import {
    computeROI,
    normalizeBoundaries,
} from '../../utils/boundaryUtils';
import type {
    Boundaries,
    BoundaryKey,
    FieldBoundariesConfig,
} from '../../utils/boundaryUtils';
import './BoundaryEditor.css';
import { Loader2, AlertCircle } from 'lucide-react';

interface BoundaryEditorProps {
    runId: string;
    onComplete: () => void;
    onCancel: () => void;
}

const BoundaryEditor: React.FC<BoundaryEditorProps> = ({ runId, onComplete, onCancel }) => {
    const [frames, setFrames] = useState<string[]>([]);
    const [currentFrameIndex, setCurrentFrameIndex] = useState(0);
    const [activeBoundary, setActiveBoundary] = useState<BoundaryKey>('field');
    const [mode, setMode] = useState<EditorMode>('add');
    const [boundaries, setBoundaries] = useState<Boundaries>({
        field: [],
        left_end_zone: [],
        right_end_zone: [],
    });
    const [imageSize, setImageSize] = useState({ width: 0, height: 0 });
    const [isLoading, setIsLoading] = useState(true);
    const [isSaving, setIsSaving] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const loadFrames = React.useCallback(async () => {
        setIsLoading(true);
        setError(null);
        try {
            const res = await fetch(`/api/runs/${runId}/calibration/list`);
            let list = await res.json();

            if (list.length === 0) {
                // Try extracting frames
                const extractRes = await fetch(`/api/runs/${runId}/calibration/extract`, { method: 'POST' });
                if (!extractRes.ok) throw new Error("Failed to extract calibration frames");
                list = await extractRes.json();
            }

            setFrames(list);
        } catch (err) {
            console.error(err);
            setError("Failed to load calibration frames. Ensure the video exists.");
        } finally {
            setIsLoading(false);
        }
    }, [runId]);

    useEffect(() => {
        loadFrames();
    }, [loadFrames]);

    const roi = useMemo(() => {
        return computeROI(boundaries, imageSize.width, imageSize.height);
    }, [boundaries, imageSize]);

    const handleSave = async () => {
        // Validation
        for (const [key, pts] of Object.entries(boundaries)) {
            if (pts.length < 3) {
                alert(`Boundary "${key}" must have at least 3 points.`);
                return;
            }
        }

        if (!roi) {
            alert("Please define boundaries before saving.");
            return;
        }

        setIsSaving(true);
        try {
            const normalized = normalizeBoundaries(boundaries, roi);
            const config: FieldBoundariesConfig = {
                ...normalized,
                roi,
            };

            // Save boundaries and ROI (consolidated)
            const saveRes = await fetch(`/api/runs/${runId}/calibration/boundaries`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(config),
            });

            if (!saveRes.ok) throw new Error("Failed to save field boundaries");

            onComplete();
        } catch (err) {
            console.error("Save failed:", err);
            alert("Failed to save configuration.");
        } finally {
            setIsSaving(false);
        }
    };

    if (isLoading) {
        return (
            <div className="empty-state" style={{ padding: '4rem' }}>
                <Loader2 className="animate-spin" size={32} color="var(--accent-primary)" />
                <p style={{ marginTop: '1rem' }}>Preparing calibration frames...</p>
            </div>
        );
    }

    if (error) {
        return (
            <div className="empty-state" style={{ padding: '4rem' }}>
                <AlertCircle size={32} color="#ef4444" />
                <p style={{ marginTop: '1rem', color: '#ef4444' }}>{error}</p>
                <button className="btn btn-secondary" onClick={onCancel} style={{ marginTop: '1.5rem' }}>
                    Back to Run Details
                </button>
            </div>
        );
    }

    return (
        <div className="boundary-editor">
            <div className="editor-header nav" style={{ border: 'none', marginBottom: 0 }}>
                <div>
                    <h1>📐 Field Boundary Setup</h1>
                    <p>Define the field zones to start processing.</p>
                </div>
                <div style={{ display: 'flex', gap: '0.75rem' }}>
                    <button className="btn btn-secondary" onClick={onCancel}>Cancel</button>
                    <button
                        className="btn btn-primary"
                        onClick={handleSave}
                        disabled={isSaving}
                    >
                        {isSaving ? <Loader2 className="animate-spin" size={18} /> : null}
                        Confirm & Save
                    </button>
                </div>
            </div>

            <div className="editor-layout">
                <div className="editor-main">
                    <BoundaryControls
                        frames={frames}
                        currentIndex={currentFrameIndex}
                        onFrameChange={setCurrentFrameIndex}
                        activeBoundary={activeBoundary}
                        onBoundaryChange={setActiveBoundary}
                        mode={mode}
                        onModeChange={setMode}
                    />
                    <BoundaryCanvas
                        imageUrl={`/api/runs/${runId}/calibration/frame/${frames[currentFrameIndex]}`}
                        boundaries={boundaries}
                        activeBoundary={activeBoundary}
                        mode={mode}
                        onBoundariesChange={setBoundaries}
                        onImageLoad={(w, h) => setImageSize({ width: w, height: h })}
                        roi={roi}
                    />
                </div>

                <div className="editor-sidebar">
                    <PointsList
                        points={boundaries[activeBoundary]}
                        onPointsChange={(newPoints) => setBoundaries(prev => ({ ...prev, [activeBoundary]: newPoints }))}
                    />
                </div>
            </div>
        </div>
    );
};

export default BoundaryEditor;
