import React, { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import BoundaryEditor from "../components/BoundaryEditor/BoundaryEditor";
import RunHeader from "../components/RunDetails/RunHeader";
import PropertiesCard from "../components/RunDetails/PropertiesCard";
import DependenciesCard from "../components/RunDetails/DependenciesCard";
import ProcessingCard from "../components/RunDetails/ProcessingCard";
import { useRunDetails } from "../hooks/useRunDetails";
import { useVideoProcessing } from "../hooks/useVideoProcessing";

const RunDetailsPage: React.FC = () => {
  const { id } = useParams();
  const navigate = useNavigate();
  const [showBoundaryEditor, setShowBoundaryEditor] = useState(false);

  const {
    run,
    loading,
    isEditing,
    setIsEditing,
    editName,
    setEditName,
    editTeamSize,
    setEditTeamSize,
    editLightTeamName,
    setEditLightTeamName,
    editDarkTeamName,
    setEditDarkTeamName,
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
  } = useVideoProcessing(id, run, fetchRun);

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

          <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
            <div className="lg:col-span-2 space-y-8">
              <PropertiesCard
                run={run}
                isEditing={isEditing}
                editTeamSize={editTeamSize}
                setEditTeamSize={setEditTeamSize}
                editLightTeamName={editLightTeamName}
                setEditLightTeamName={setEditLightTeamName}
                editDarkTeamName={editDarkTeamName}
                setEditDarkTeamName={setEditDarkTeamName}
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
                handleStartProcessing={handleStartProcessing}
                handleStopProcessing={handleStopProcessing}
              />
            </div>
          </div>
        </>
      )}
    </div>
  );
};

export default RunDetailsPage;
