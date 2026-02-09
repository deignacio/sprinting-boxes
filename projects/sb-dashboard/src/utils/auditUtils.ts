// Audit utility functions for score calculation and break detection

export interface CliffData {
  frame_index: number;
  timestamp: string;
  left_emptied_first: boolean;
  right_emptied_first: boolean;
  maybe_false_positive: boolean;
  status: "Unconfirmed" | "Confirmed" | "FalsePositive";
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
  time_offset_secs: number;
  video_start_time: string;
}

export function recalculateAudit(
  cliffs: CliffData[],
  settings: AuditSettings,
): CliffData[] {
  if (!cliffs || cliffs.length === 0) return [];

  const initialScoreLight = parseInt(String(settings.initial_score_light)) || 0;
  const initialScoreDark = parseInt(String(settings.initial_score_dark)) || 0;

  let scoreLight = initialScoreLight;
  let scoreDark = initialScoreDark;

  const sorted = [...cliffs].sort((a, b) => a.frame_index - b.frame_index);
  const result: CliffData[] = [];
  let lastValidLeftColor: string | null = null;

  // Pass 1: Core Scoring and Team Assignment
  for (let i = 0; i < sorted.length; i++) {
    const cliff = sorted[i];
    const isFP = cliff.status === "FalsePositive";

    if (isFP) {
      result.push({
        ...cliff,
        left_team_color: undefined,
        right_team_color: undefined,
        score_light: scoreLight,
        score_dark: scoreDark,
        is_break: false,
      });
      continue;
    }

    // Determine team colors for this point
    let left = cliff.left_team_color;
    let right = cliff.right_team_color;

    if (left) {
      // Manual override exists for this point or it's been set already
      right = left === "light" ? "dark" : "light";
    } else if (lastValidLeftColor === null) {
      // First valid point: default to light on left
      left = "light";
      right = "dark";
    } else {
      // Subsequent points: toggle from last valid point
      left = lastValidLeftColor === "light" ? "dark" : "light";
      right = left === "light" ? "dark" : "light";
    }

    lastValidLeftColor = left;

    // Score update: if not first point
    if (i > 0) {
      let pullSide = "unknown";
      if (cliff.manual_side_override) pullSide = cliff.manual_side_override;
      else if (cliff.left_emptied_first) pullSide = "left";
      else if (cliff.right_emptied_first) pullSide = "right";

      if (pullSide !== "unknown") {
        const pullingTeam = pullSide === "left" ? left : right;
        if (pullingTeam === "light") scoreLight++;
        else if (pullingTeam === "dark") scoreDark++;
      }
    }

    result.push({
      ...cliff,
      left_team_color: left,
      right_team_color: right,
      score_light: scoreLight,
      score_dark: scoreDark,
      is_break: false,
    });
  }

  // Pass 2: Break Detection
  const validPoints = result.filter((c) => c.status !== "FalsePositive");
  for (let j = 0; j < validPoints.length - 1; j++) {
    const cur = validPoints[j];
    const next = validPoints[j + 1];

    const curPullSide =
      cur.manual_side_override || (cur.left_emptied_first ? "left" : "right");
    const curPullTeam =
      curPullSide === "left" ? cur.left_team_color : cur.right_team_color;

    const nextPullSide =
      next.manual_side_override || (next.left_emptied_first ? "left" : "right");
    const nextPullTeam =
      nextPullSide === "left" ? next.left_team_color : next.right_team_color;

    if (curPullTeam && nextPullTeam && curPullTeam === nextPullTeam) {
      cur.is_break = true;
    }
  }

  return result;
}
