import { useState, useCallback, useEffect, useRef } from "react";
import type { ProcessingProgress, RunDetail } from "../types/run";

export const useVideoProcessing = (
  id: string | undefined,
  run: RunDetail | null,
  fetchRun: () => void,
) => {
  const [isProcessing, setIsProcessing] = useState(false);
  const [processingProgress, setProcessingProgress] =
    useState<ProcessingProgress | null>(null);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  const connectToSSE = useCallback(() => {
    if (!id || eventSourceRef.current) return;

    const eventSource = new EventSource(`/api/runs/${id}/process/progress/sse`);
    eventSourceRef.current = eventSource;

    eventSource.onmessage = (event) => {
      try {
        if (event.data === "keep-alive") return;
        const data = JSON.parse(event.data) as
          | ProcessingProgress
          | { error: string };

        if ("error" in data && data.error) {
          setProcessingError(data.error);
          setIsProcessing(false);
        } else {
          setProcessingProgress(data as ProcessingProgress);
          if ((data as ProcessingProgress).is_complete) {
            setIsProcessing(false);
          }
        }
      } catch (e) {
        console.error("Failed to parse SSE data:", e);
      }
    };

    eventSource.onerror = (err) => {
      console.error("SSE connection error:", err);
      setIsProcessing(false);
    };
  }, [id]);

  useEffect(() => {
    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }
    };
  }, []);

  // Manage connection based on isProcessing state
  useEffect(() => {
    if (isProcessing) {
      connectToSSE();
    } else if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, [isProcessing, connectToSSE]);

  // Check for existing processing on load
  useEffect(() => {
    if (id) {
      fetch(`/api/runs/${id}/process/progress`)
        .then((res) => {
          if (res.ok) return res.json();
          return null;
        })
        .then((data: ProcessingProgress | null) => {
          if (data && data.is_active) {
            console.log("Detected active processing on load");
            setIsProcessing(true);
            setProcessingProgress(data);
          }
        })
        .catch(() => {
          /* not processing */
        });
    }
  }, [id]);

  const handleComputeCrops = async () => {
    if (!run) return;
    try {
      const response = await fetch(`/api/runs/${run.run_id}/crops/compute`, {
        method: "POST",
      });
      if (!response.ok) {
        throw new Error("Failed to compute crops configuration");
      }
      fetchRun();
    } catch (e) {
      console.error("Failed to compute crops:", e);
      setProcessingError(
        e instanceof Error ? e.message : "Failed to compute crops",
      );
    }
  };

  const handleStartProcessing = async (backend: string = "opencv") => {
    if (!run) return;
    setProcessingError(null);

    // Don't set isProcessing=true yet to avoid race condition with SSE
    setProcessingProgress({
      run_id: run.run_id,
      total_frames: 0,
      is_active: true,
      is_complete: false,
      error: null,
      stages: {},
    });

    try {
      console.log("Starting processing via API...");
      const response = await fetch(`/api/runs/${run.run_id}/process/start`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ backend }),
      });

      if (!response.ok) {
        if (response.status === 412) {
          throw new Error("Dependencies not met");
        }
        throw new Error("Failed to start processing");
      }

      const data = await response.json();
      console.log("Processing started successfully:", data);
      setProcessingProgress(data);
      setIsProcessing(true); // Now we can safely connect to SSE
    } catch (e) {
      console.error("Failed to start processing:", e);
      setProcessingError(e instanceof Error ? e.message : "Unknown error");
      setIsProcessing(false);
    }
  };

  const handleStopProcessing = async () => {
    if (!run) return;
    try {
      await fetch(`/api/runs/${run.run_id}/process/stop`, {
        method: "POST",
      });
      setIsProcessing(false);
    } catch (e) {
      console.error("Failed to stop processing:", e);
    }
  };

  const handleUpdateWorkers = async (stage: string, delta: number) => {
    if (!run) return;
    try {
      await fetch(`/api/runs/${run.run_id}/process/workers`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ delta, stage }),
      });
    } catch (e) {
      console.error("Failed to update workers", e);
    }
  };

  return {
    isProcessing,
    processingProgress,
    processingError,
    handleComputeCrops,
    handleStartProcessing,
    handleStopProcessing,
    handleUpdateWorkers,
  };
};
