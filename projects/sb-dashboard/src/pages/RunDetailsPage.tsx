import React, { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import BoundaryEditor from "../components/BoundaryEditor/BoundaryEditor";
import RunHeader from "../components/RunDetails/RunHeader";
import PropertiesCard from "../components/RunDetails/PropertiesCard";
import DependenciesCard from "../components/RunDetails/DependenciesCard";
import ProcessingCard from "../components/RunDetails/ProcessingCard";
import AuditView from "../components/RunDetails/AuditView";
import CliffDetail from "../components/RunDetails/CliffDetail";
import FrameViewer from "../components/RunDetails/FrameViewer";
import { useRunDetails } from "../hooks/useRunDetails";
import { useVideoProcessing } from "../hooks/useVideoProcessing";
import { type CliffData, type AuditSettings } from "../utils/auditUtils";

type ViewState = "overview" | "audit" | "cliff_detail" | "frame_viewer";

const RunDetailsPage: React.FC = () => {
  const { id } = useParams();
  const navigate = useNavigate();
  const [showBoundaryEditor, setShowBoundaryEditor] = useState(false);
  const [view, setView] = useState<ViewState>("overview");
  const [selectedCliff, setSelectedCliff] = useState<CliffData | null>(null);
  const [allCliffs, setAllCliffs] = useState<CliffData[]>([]);
  const [auditSettings, setAuditSettings] = useState<AuditSettings | null>(
    null,
  );
  const [selectedBackend, setSelectedBackend] = useState("opencv");

  const {
    run,
    loading,
    isEditing,
    setIsEditing,
    editName,
    setEditName,
    editTeamSize,
    setEditTeamSize,
    editTags,
    setEditTags,
    editSampleRate,
    setEditSampleRate,
    isSaving,
    fetchRun,
    handleSave,
  } = useRunDetails(id);

  const {
    isProcessing,
    processingProgress,
    processingError,
    handleComputeCrops,
    handleStartProcessing,
    handleStopProcessing,
    handleUpdateWorkers,
  } = useVideoProcessing(id, run, fetchRun);

  // Load full audit state
  const loadAuditData = async () => {
    try {
      const response = await fetch(`/api/runs/${id}/audit/cliffs`);
      if (response.ok) {
        const data = await response.json();
        setAllCliffs(data.cliffs);
        setAuditSettings(data.settings);
      }
    } catch (err) {
      console.error("Failed to load audit data", err);
    }
  };

  const handleUpdateSettings = async (newSettings: AuditSettings) => {
    try {
      await fetch(`/api/runs/${id}/audit/settings`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(newSettings),
      });
      // Reload to get recalculated scores
      await loadAuditData();
    } catch (err) {
      console.error("Failed to update settings", err);
    }
  };

  const handleCliffClick = (cliff: CliffData) => {
    setSelectedCliff(cliff);
    setView("cliff_detail");
    // Ensure we have the full list for navigation
    if (allCliffs.length === 0) loadAuditData();
  };

  if (loading) {
    return <div className="container empty-state">Loading run details...</div>;
  }

  if (!run) {
    return (
      <div className="container empty-state">
        <h2>Run Not Found</h2>
        <button
          onClick={() => navigate("/")}
          className="btn btn-secondary"
          style={{ marginTop: "1rem" }}
        >
          Back to Dashboard
        </button>
      </div>
    );
  }

  // If in cliff detail view, show full screen component
  if (view === "cliff_detail" && selectedCliff && auditSettings) {
    return (
      <CliffDetail
        key={selectedCliff.frame_index}
        runId={id!}
        cliff={selectedCliff}
        allCliffs={allCliffs}
        settings={auditSettings}
        onUpdateSettings={handleUpdateSettings}
        onBack={() => setView("audit")}
        onNavigate={(cliff) => setSelectedCliff(cliff)}
      />
    );
  } else if (view === "cliff_detail" && !auditSettings) {
    loadAuditData(); // lazy load if missing
    return (
      <div className="container empty-state">Loading audit settings...</div>
    );
  }

  return (
    <div className="container">
      {showBoundaryEditor ? (
        <BoundaryEditor
          runId={id!}
          onComplete={() => {
            setShowBoundaryEditor(false);
            fetchRun();
          }}
          onCancel={() => setShowBoundaryEditor(false)}
        />
      ) : (
        <>
          <div
            style={{
              position: "sticky",
              top: 0,
              zIndex: 50,
              backgroundColor: "var(--bg-primary)",
              margin: "-2rem -2rem 0 -2rem", // Compensate for container padding
              padding: "1rem 2rem 0 2rem",   // Reduced top padding
              borderBottom: "1px solid #334155", // Move border here for cleanliness
            }}
          >
            <RunHeader
              run={run}
              isEditing={isEditing}
              setIsEditing={setIsEditing}
              editName={editName}
              setEditName={setEditName}
              isSaving={isSaving}
              onSave={handleSave}
              onBack={() => navigate("/")}
            />

            {/* Tab Navigation */}
            <div
              style={{
                display: "flex",
                gap: "1.5rem",
                backgroundColor: "var(--bg-primary)",
                marginTop: "0.5rem",
              }}
            >
              <button
                onClick={() => setView("overview")}
                style={{
                  padding: "0.5rem 0",
                  background: "transparent",
                  color: view === "overview" ? "#34d399" : "#94a3b8",
                  border: "none",
                  borderBottom:
                    view === "overview"
                      ? "2px solid #34d399"
                      : "2px solid transparent",
                  cursor: "pointer",
                  fontSize: "0.875rem",
                  fontWeight: view === "overview" ? 600 : 400,
                }}
              >
                Overview
              </button>
              <button
                onClick={() => {
                  setView("audit");
                  loadAuditData();
                }}
                style={{
                  padding: "0.5rem 0",
                  background: "transparent",
                  color: view === "audit" ? "#34d399" : "#94a3b8",
                  border: "none",
                  borderBottom:
                    view === "audit"
                      ? "2px solid #34d399"
                      : "2px solid transparent",
                  cursor: "pointer",
                  fontSize: "0.875rem",
                  fontWeight: view === "audit" ? 600 : 400,
                }}
              >
                Point Audit
              </button>
              <button
                onClick={() => {
                  setView("frame_viewer");
                  loadAuditData();
                }}
                style={{
                  padding: "0.5rem 0",
                  background: "transparent",
                  color: view === "frame_viewer" ? "#34d399" : "#94a3b8",
                  border: "none",
                  borderBottom:
                    view === "frame_viewer"
                      ? "2px solid #34d399"
                      : "2px solid transparent",
                  cursor: "pointer",
                  fontSize: "0.875rem",
                  fontWeight: view === "frame_viewer" ? 600 : 400,
                }}
              >
                Frame Viewer
              </button>
            </div>
          </div>

          {view === "overview" && (
            <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
              <div className="lg:col-span-2 space-y-8">
                <PropertiesCard
                  run={run}
                  isEditing={isEditing}
                  editTeamSize={editTeamSize}
                  setEditTeamSize={setEditTeamSize}
                  editTags={editTags}
                  setEditTags={setEditTags}
                  editSampleRate={editSampleRate}
                  setEditSampleRate={setEditSampleRate}
                />
              </div>

              <div className="space-y-8">
                <DependenciesCard
                  run={run}
                  setShowBoundaryEditor={setShowBoundaryEditor}
                  onComputeCrops={handleComputeCrops}
                />

                <ProcessingCard
                  run={run}
                  isProcessing={isProcessing}
                  processingProgress={processingProgress}
                  processingError={processingError}
                  selectedBackend={selectedBackend}
                  onBackendChange={setSelectedBackend}
                  handleStartProcessing={handleStartProcessing}
                  handleStopProcessing={handleStopProcessing}
                  handleUpdateWorkers={handleUpdateWorkers}
                />
              </div>
            </div>
          )}


          {view === "audit" && (
            <AuditView runId={id!} onCliffClick={handleCliffClick} />
          )}

          {view === "frame_viewer" && (
            !auditSettings ? (
              <div className="p-8 text-center text-slate-400">Loading audit data...</div>
            ) : (
              <FrameViewer
                runId={id!}
                allCliffs={allCliffs}
                settings={auditSettings}
              />
            )
          )}
        </>
      )}
    </div>
  );
};
export default RunDetailsPage;
