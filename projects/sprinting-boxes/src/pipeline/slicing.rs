use anyhow::Result;
use opencv::core::{Mat, Rect, Scalar};
use opencv::prelude::*;

/// Configuration for sliding window inference
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SliceConfig {
    /// Size of each square tile (e.g., 640)
    pub tile_size: u32,
    /// Overlap between tiles as fraction (0.0-0.5)
    pub overlap: f32,
    /// IoU threshold for NMS deduplication
    pub nms_iou_threshold: f32,
}

impl SliceConfig {
    pub fn new(tile_size: u32, overlap: f32) -> Self {
        Self {
            tile_size,
            overlap: overlap.clamp(0.0, 0.5),
            nms_iou_threshold: 0.5,
        }
    }

    /// Returns true if slicing is enabled (tile_size > 0)
    pub fn is_enabled(&self) -> bool {
        self.tile_size > 0
    }

    /// Calculate stride between tiles based on overlap
    pub fn stride(&self) -> u32 {
        ((self.tile_size as f32) * (1.0 - self.overlap)) as u32
    }
}

impl Default for SliceConfig {
    fn default() -> Self {
        Self {
            tile_size: 0, // Disabled by default
            overlap: 0.2,
            nms_iou_threshold: 0.5,
        }
    }
}

/// A tile extracted from a larger image
#[derive(Clone)]
pub struct Tile {
    /// The tile image (padded to tile_size x tile_size)
    pub image: Mat,
    /// X offset of this tile in the original image
    pub x_offset: i32,
    /// Y offset of this tile in the original image
    pub y_offset: i32,
    /// Original width before padding
    #[allow(dead_code)]
    pub original_width: i32,
    /// Original height before padding
    #[allow(dead_code)]
    pub original_height: i32,
}

/// Helper to generate tile offsets along one dimension
fn generate_offsets(total_size: i32, tile_size: i32, stride: i32) -> Vec<i32> {
    let mut offsets = Vec::new();
    let mut pos = 0;

    // Generate standard strided tiles
    while pos < total_size {
        offsets.push(pos);

        // If this tile touches or crosses the edge, we stop standard generation
        if pos + tile_size >= total_size {
            break;
        }
        pos += stride;
    }

    // Check if we need a final edge-aligned tile to avoid black padding
    if let Some(&last) = offsets.last() {
        if last + tile_size > total_size && total_size >= tile_size {
            let edge_aligned = total_size - tile_size;
            if edge_aligned != last {
                offsets.push(edge_aligned);
            }
        }
    }

    offsets.sort();
    offsets.dedup();
    offsets
}

/// Generate overlapping tiles from an image
pub fn generate_tiles(image: &Mat, config: &SliceConfig) -> Result<Vec<Tile>> {
    if !config.is_enabled() {
        return Ok(vec![]);
    }

    let size = image.size()?;
    let img_w = size.width;
    let img_h = size.height;
    let tile_size = config.tile_size as i32;
    let stride = config.stride() as i32;

    let x_offsets = generate_offsets(img_w, tile_size, stride);
    let y_offsets = generate_offsets(img_h, tile_size, stride);

    let mut tiles = Vec::new();

    for &tile_y in &y_offsets {
        for &tile_x in &x_offsets {
            let tile_w = (tile_size).min(img_w - tile_x);
            let tile_h = (tile_size).min(img_h - tile_y);

            let roi = Rect::new(tile_x, tile_y, tile_w, tile_h);
            let cropped_roi = Mat::roi(image, roi)?;

            let mut cropped = Mat::default();
            cropped_roi.copy_to(&mut cropped)?;

            let padded = if tile_w < tile_size || tile_h < tile_size {
                let bottom = tile_size - tile_h;
                let right = tile_size - tile_w;
                let mut padded = Mat::default();
                opencv::core::copy_make_border(
                    &cropped,
                    &mut padded,
                    0,      // top
                    bottom, // bottom
                    0,      // left
                    right,  // right
                    opencv::core::BORDER_CONSTANT,
                    Scalar::all(0.0),
                )?;
                padded
            } else {
                cropped
            };

            tiles.push(Tile {
                image: padded,
                x_offset: tile_x,
                y_offset: tile_y,
                original_width: tile_w,
                original_height: tile_h,
            });
        }
    }

    Ok(tiles)
}

/// Transform a detection from tile coordinates to original image coordinates
pub fn transform_detection_to_image_coords(detection: &usls::Hbb, tile: &Tile) -> usls::Hbb {
    let x = detection.xmin() + tile.x_offset as f32;
    let y = detection.ymin() + tile.y_offset as f32;
    let w = detection.width();
    let h = detection.height();

    let mut new_hbb = usls::Hbb::default().with_xyxy(x, y, x + w, y + h);

    if let Some(conf) = detection.confidence() {
        new_hbb = new_hbb.with_confidence(conf);
    }
    if let Some(id) = detection.id() {
        new_hbb = new_hbb.with_id(id);
    }
    if let Some(name) = detection.name() {
        new_hbb = new_hbb.with_name(name);
    }

    new_hbb
}

/// Apply Non-Maximum Suppression to remove duplicate detections
pub fn nms(detections: Vec<usls::Hbb>, iou_threshold: f32) -> Vec<usls::Hbb> {
    if detections.is_empty() {
        return detections;
    }

    // Sort by confidence (highest first)
    let mut sorted: Vec<_> = detections.into_iter().collect();
    sorted.sort_by(|a, b| {
        let conf_a = a.confidence().unwrap_or(0.0);
        let conf_b = b.confidence().unwrap_or(0.0);
        conf_b
            .partial_cmp(&conf_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut keep = Vec::new();
    let mut suppressed = vec![false; sorted.len()];

    for i in 0..sorted.len() {
        if suppressed[i] {
            continue;
        }

        keep.push(sorted[i].clone());

        for j in (i + 1)..sorted.len() {
            if suppressed[j] {
                continue;
            }

            let iou = compute_iou(&sorted[i], &sorted[j]);
            if iou > iou_threshold {
                suppressed[j] = true;
            }
        }
    }

    keep
}

/// Compute Intersection over Union between two bounding boxes
fn compute_iou(a: &usls::Hbb, b: &usls::Hbb) -> f32 {
    let x1 = a.xmin().max(b.xmin());
    let y1 = a.ymin().max(b.ymin());
    let x2 = (a.xmin() + a.width()).min(b.xmin() + b.width());
    let y2 = (a.ymin() + a.height()).min(b.ymin() + b.height());

    if x2 <= x1 || y2 <= y1 {
        return 0.0;
    }

    let intersection = (x2 - x1) * (y2 - y1);
    let area_a = a.width() * a.height();
    let area_b = b.width() * b.height();
    let union = area_a + area_b - intersection;

    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}
