// Audit utility functions for the sprinting-boxes dashboard
//
// IMPORTANT: The core scoring logic now lives in the backend (Rust).
// This module provides:
// - TypeScript interfaces matching the backend models
// - API functions to interact with the backend
// - Any frontend-only utility functions

export interface CliffData {
  frame_index: number;
  timestamp: string;
  left_emptied_first: boolean;
  right_emptied_first: boolean;
  maybe_false_positive: boolean;
  status: "Unconfirmed" | "Confirmed" | "FalsePositive" | "Halftime";
  halftime_winner?: "light" | "dark" | null;
  manual_side_override?: "left" | "right";
  manual_color_override?: "light" | "dark";
  left_team_color?: "light" | "dark";
  right_team_color?: "light" | "dark";
  score_light: number;
  score_dark: number;
  is_break: boolean;
}

export interface AuditSettings {
  light_team_name: string;
  dark_team_name: string;
  initial_score_light: number;
  initial_score_dark: number;
  video_start_time: string;
}

export interface AuditState {
  cliffs: CliffData[];
  settings: AuditSettings;
}

/**
 * Recalculate audit by calling the backend API
 * 
 * This replaces the previous local recalculation logic.
 * The backend handles all scoring, team assignment, and break detection.
 */
export async function recalculateAudit(
  cliffs: CliffData[],
  settings: AuditSettings,
  runId: string
): Promise<CliffData[]> {
  const response = await fetch(`/api/runs/${runId}/audit/recalculate`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ cliffs, settings }),
  });
  
  if (!response.ok) {
    throw new Error('Failed to recalculate audit');
  }
  
  const data: AuditState = await response.json();
  return data.cliffs;
}

/**
 * Load audit data from the backend
 */
export async function loadAuditData(runId: string): Promise<AuditState> {
  const response = await fetch(`/api/runs/${runId}/audit/cliffs`);
  
  if (!response.ok) {
    throw new Error('Failed to load audit data');
  }
  
  return response.json();
}

/**
 * Save audit data to the backend
 */
export async function saveAuditData(
  runId: string,
  cliffs: CliffData[],
  settings: AuditSettings
): Promise<void> {
  const response = await fetch(`/api/runs/${runId}/audit/cliffs`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ cliffs, settings }),
  });
  
  if (!response.ok) {
    throw new Error('Failed to save audit data');
  }
}

/**
 * Update audit settings on the backend
 */
export async function updateAuditSettings(
  runId: string,
  settings: AuditSettings
): Promise<void> {
  const response = await fetch(`/api/runs/${runId}/audit/settings`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(settings),
  });
  
  if (!response.ok) {
    throw new Error('Failed to update settings');
  }
}

/**
 * Update a single cliff field
 */
export async function updateCliffField(
  runId: string,
  frameIndex: number,
  field: string
): Promise<void> {
  const response = await fetch(
    `/api/runs/${runId}/audit/cliffs/${frameIndex}/${field}`,
    { method: 'POST' }
  );
  
  if (!response.ok) {
    throw new Error('Failed to update cliff field');
  }
}
