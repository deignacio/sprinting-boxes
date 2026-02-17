export interface RunDependency {
  artifact_name: string;
  message: string;
  valid: boolean;
}

export interface RunContext {
  original_name: string;
  display_name: string;
  created_at: string;
  run_id: string;
  team_size: number;
  light_team_name: string;
  dark_team_name: string;
  tags: string[];
  sample_rate: number;
}

export interface RunDetail {
  run_id: string;
  run_context: RunContext;
  missing_dependencies: RunDependency[];
}

export interface StageProgress {
  current: number;
  total: number;
  ms_per_frame: number;
  fps?: number;
}

export interface ProcessingProgress {
  run_id: string;
  total_frames: number;
  is_active: boolean;
  is_complete: boolean;
  error: string | null;
  stages: Record<string, StageProgress>;
  active_reader_workers?: number;
  active_crop_workers?: number;
  active_detect_workers?: number;
  processing_rate?: number;
  effective_fps?: number;
}
