pub mod cliff;
pub mod gpu;
pub mod pull_side;
pub mod scoring;

pub use cliff::{is_cliff_at, CliffDetector, CliffDetectorConfig};
pub use gpu::GpuCliffDetector;
pub use pull_side::{detect_pull_side, PullSide};
pub use scoring::{pre_point_score, EndZoneOccupancy};
