// Run artifact struct definitions
//
// This module contains the struct definitions for artifacts that are persisted
// as JSON files within a run's output directory.

use serde::{Deserialize, Serialize};

/// A 2D point in normalized coordinates [0, 1]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Normalized bounding box
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct BBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// ROI definition (optional, embedded in field_boundaries.json)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ROIDefinition {
    pub x_normalized: f32,
    pub y_normalized: f32,
    pub width_normalized: f32,
    pub height_normalized: f32,
}

/// Field boundaries as defined in field_boundaries.json
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FieldBoundaries {
    pub field: Vec<Point>,
    pub left_end_zone: Vec<Point>,
    pub right_end_zone: Vec<Point>,
    #[serde(default)]
    pub roi: Option<ROIDefinition>,
}

/// A single crop configuration for a boundary region
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CropConfigData {
    pub name: String,
    pub bbox: BBox,
    pub original_polygon: Vec<Point>,
    pub effective_polygon: Vec<Point>,
}

/// Collection of all crop configs for a run
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CropsConfig {
    pub left_end_zone: CropConfigData,
    pub right_end_zone: CropConfigData,
}

impl FieldBoundaries {
    /// Transforms points from ROI-relative to global normalized coordinates.
    pub fn get_global_points(&self, points: &[Point]) -> Vec<Point> {
        points
            .iter()
            .map(|p| {
                if let Some(ref roi) = self.roi {
                    Point {
                        x: roi.x_normalized + (p.x * roi.width_normalized),
                        y: roi.y_normalized + (p.y * roi.height_normalized),
                    }
                } else {
                    *p
                }
            })
            .collect()
    }
}
