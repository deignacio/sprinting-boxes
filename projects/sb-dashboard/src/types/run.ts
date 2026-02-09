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

export interface ProcessingProgress {
    run_id: string;
    frames_read: number;
    frames_processed: number;
    total_frames: number;
    is_active: boolean;
    is_complete: boolean;
    error: string | null;
}
