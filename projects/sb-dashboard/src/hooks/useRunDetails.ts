import { useState, useCallback, useEffect } from "react";
import type { RunDetail } from "../types/run";

export const useRunDetails = (id: string | undefined) => {
  const [run, setRun] = useState<RunDetail | null>(null);
  const [loading, setLoading] = useState(true);

  // Edit State
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState("");
  const [editTeamSize, setEditTeamSize] = useState(7);
  const [editTags, setEditTags] = useState("");
  const [editSampleRate, setEditSampleRate] = useState(1.0);
  const [isSaving, setIsSaving] = useState(false);

  const fetchRun = useCallback(() => {
    if (!id) return;
    setLoading(true);
    fetch(`/api/runs/${id}`)
      .then((res) => {
        if (!res.ok) throw new Error("Run not found");
        return res.json();
      })
      .then((data: RunDetail) => {
        setRun(data);
        setEditName(data.run_context.display_name);
        setEditTeamSize(data.run_context.team_size);
        setEditTags(data.run_context.tags.join(", "));
        setEditSampleRate(data.run_context.sample_rate || 1.0);
        setLoading(false);
      })
      .catch((err) => {
        console.error(err);
        setLoading(false);
      });
  }, [id]);

  useEffect(() => {
    fetchRun();
  }, [fetchRun]);

  const handleSave = async () => {
    if (!run) return;
    setIsSaving(true);

    const updatedRunContext = {
      ...run.run_context,
      display_name: editName,
      team_size: editTeamSize,
      tags: editTags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0),
      sample_rate: editSampleRate,
    };

    try {
      const response = await fetch(`/api/runs/${run.run_id}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(updatedRunContext),
      });

      if (!response.ok) throw new Error("Failed to update run");

      const newRunContext = await response.json();
      setRun({ ...run, run_context: newRunContext });
      setIsEditing(false);
    } catch (err) {
      console.error(err);
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  return {
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
  };
};
