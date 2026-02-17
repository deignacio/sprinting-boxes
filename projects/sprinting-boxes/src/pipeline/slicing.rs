use crate::pipeline::types::Point;
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
    if total_size <= tile_size {
        return vec![0];
    }

    let mut offsets = Vec::new();
    let limit = total_size - tile_size;

    // Heuristic: If the image is only slightly larger than one tile (e.g. < 25% extra),
    // just use ONE centered tile to avoid 90%+ redundancy.
    if total_size < (tile_size + tile_size / 4) {
        return vec![limit / 2];
    }

    let mut pos = 0;
    while pos < limit {
        offsets.push(pos);
        pos += stride;
    }

    // Handle the final edge-aligned tile
    if let Some(&last) = offsets.last() {
        if last < limit {
            // Only merge with the edge if we have more than one offset
            // AND the gap is small. This prevents popping the '0' offset
            // for dimensions only slightly larger than tile_size.
            if offsets.len() > 1 && (limit - last) < (stride / 2) {
                offsets.pop();
            }
            offsets.push(limit);
        }
    } else {
        offsets.push(limit);
    }

    offsets.sort();
    offsets.dedup();
    offsets
}

/// Generate overlapping tiles from an image, optionally filtering by regions of interest.
pub fn generate_tiles(
    image: &Mat,
    config: &SliceConfig,
    regions: Option<&[Vec<Point>]>,
) -> Result<Vec<Tile>> {
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

            let tile = Tile {
                image: padded,
                x_offset: tile_x,
                y_offset: tile_y,
                original_width: tile_w,
                original_height: tile_h,
            };

            let keep = if let Some(polys) = regions {
                polys
                    .iter()
                    .any(|poly| is_tile_overlapping_polygon(&tile, poly))
            } else {
                true
            };

            if keep {
                tiles.push(tile);
            }
        }
    }

    Ok(tiles)
}

/// Check if a tile overlaps with a polygon
fn is_tile_overlapping_polygon(tile: &Tile, poly: &[Point]) -> bool {
    if poly.is_empty() {
        return false;
    }

    // 1. Fast path: Bounding box check
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for p in poly {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let tile_right = (tile.x_offset + tile.original_width) as f32;
    let tile_bottom = (tile.y_offset + tile.original_height) as f32;

    if max_x < tile.x_offset as f32
        || min_x > tile_right
        || max_y < tile.y_offset as f32
        || min_y > tile_bottom
    {
        return false;
    }

    // 2. Check if any polygon vertex is inside the tile
    for p in poly {
        if p.x >= tile.x_offset as f32
            && p.x <= tile_right
            && p.y >= tile.y_offset as f32
            && p.y <= tile_bottom
        {
            return true;
        }
    }

    // 3. Check if any tile vertex is inside the polygon
    let tile_vertices = [
        Point {
            x: tile.x_offset as f32,
            y: tile.y_offset as f32,
        },
        Point {
            x: tile_right,
            y: tile.y_offset as f32,
        },
        Point {
            x: tile_right,
            y: tile_bottom,
        },
        Point {
            x: tile.x_offset as f32,
            y: tile_bottom,
        },
    ];

    for &v in &tile_vertices {
        if crate::pipeline::geometry::is_point_in_polygon_robust(v.x, v.y, poly) {
            return true;
        }
    }

    // 4. Check for edge intersections
    for i in 0..poly.len() {
        let p1 = poly[i];
        let p2 = poly[(i + 1) % poly.len()];

        for j in 0..4 {
            let v1 = tile_vertices[j];
            let v2 = tile_vertices[(j + 1) % 4];

            if segments_intersect(p1, p2, v1, v2) {
                return true;
            }
        }
    }

    false
}

fn segments_intersect(p1: Point, p2: Point, p3: Point, p4: Point) -> bool {
    fn ccw(a: Point, b: Point, c: Point) -> bool {
        (c.y - a.y) * (b.x - a.x) > (b.y - a.y) * (c.x - a.x)
    }
    ccw(p1, p3, p4) != ccw(p2, p3, p4) && ccw(p1, p2, p3) != ccw(p1, p2, p4)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::types::Point;

    #[test]
    fn test_is_tile_overlapping_polygon() {
        let tile = Tile {
            image: Mat::default(),
            x_offset: 100,
            y_offset: 100,
            original_width: 100,
            original_height: 100,
        };

        // 1. Polygon entirely inside tile
        let poly1 = vec![
            Point { x: 120.0, y: 120.0 },
            Point { x: 180.0, y: 120.0 },
            Point { x: 180.0, y: 180.0 },
        ];
        assert!(is_tile_overlapping_polygon(&tile, &poly1));

        // 2. Polygon entirely outside tile
        let poly2 = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 50.0, y: 0.0 },
            Point { x: 50.0, y: 50.0 },
        ];
        assert!(!is_tile_overlapping_polygon(&tile, &poly2));

        // 3. Polygon overlaps tile
        let poly3 = vec![
            Point { x: 50.0, y: 50.0 },
            Point { x: 150.0, y: 150.0 },
            Point { x: 50.0, y: 150.0 },
        ];
        assert!(is_tile_overlapping_polygon(&tile, &poly3));

        // 4. Tile entirely inside polygon
        let poly4 = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 300.0, y: 0.0 },
            Point { x: 300.0, y: 300.0 },
            Point { x: 0.0, y: 300.0 },
        ];
        assert!(is_tile_overlapping_polygon(&tile, &poly4));
    }

    #[test]
    fn test_generate_tiles_regional() {
        let config = SliceConfig::new(100, 0.0);
        // Create a 300x300 black image
        let image =
            Mat::new_rows_cols_with_default(300, 300, opencv::core::CV_8UC3, Scalar::all(0.0))
                .unwrap();

        // Without regions (should generate 3x3 = 9 tiles)
        let tiles_all = generate_tiles(&image, &config, None).unwrap();
        assert_eq!(tiles_all.len(), 9);

        // With region at top-left (should only generate tiles overlapping 0-100, 0-100)
        let region = vec![
            Point { x: 10.0, y: 10.0 },
            Point { x: 50.0, y: 10.0 },
            Point { x: 50.0, y: 50.0 },
        ];
        let tiles_reg = generate_tiles(&image, &config, Some(&[region])).unwrap();
        // Should be just 1 tile (the first one)
        assert_eq!(tiles_reg.len(), 1);
        assert_eq!(tiles_reg[0].x_offset, 0);
        assert_eq!(tiles_reg[0].y_offset, 0);
    }

    #[test]
    fn test_hbb_name_lifetime() {
        let mut d1_transformed = {
            let mut d1 = usls::Hbb::default()
                .with_xyxy(10.0, 10.0, 50.0, 50.0)
                .with_confidence(0.9);
            d1 = d1.with_name("test_name");

            // Simulating transform_detection_to_image_coords
            let mut new_hbb = usls::Hbb::default().with_xyxy(10.0, 10.0, 50.0, 50.0);
            if let Some(name) = d1.name() {
                new_hbb = new_hbb.with_name(name);
            }
            new_hbb
            // d1 is dropped here. If new_hbb only has a pointer to d1's name, it's dangling.
        };

        // Try to access or clone it
        let _cloned = d1_transformed.clone();
        if let Some(name) = d1_transformed.name() {
            println!("Name: {}", name);
        }
    }

    #[test]
    fn test_nms_basic() {
        let d1 = usls::Hbb::default()
            .with_xyxy(10.0, 10.0, 50.0, 50.0)
            .with_confidence(0.9);
        let d2 = usls::Hbb::default()
            .with_xyxy(15.0, 15.0, 55.0, 55.0)
            .with_confidence(0.8);
        let result = nms(vec![d1, d2], 0.5);
        assert_eq!(result.len(), 1);
    }
}
