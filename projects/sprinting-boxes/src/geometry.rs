use crate::pipeline::types::BBox as PipelineBBox;
use crate::run_artifacts::{BBox, Point};
use geo::BooleanOps;
use geo::Contains;
use geo_buffer::buffer_polygon;
use geo_types::{LineString, MultiPolygon, Point as GeoPoint, Polygon};

/// Convert our pipeline points to a geo_types Polygon
fn to_geo_polygon(points: &[Point]) -> Polygon<f64> {
    let coords: Vec<(f64, f64)> = points.iter().map(|p| (p.x as f64, p.y as f64)).collect();
    let ls = LineString::from(coords);
    Polygon::new(ls, vec![])
}

/// Convert geo_types MultiPolygon back to our Points.
/// Computes the convex hull of all vertices to produce a single simple
/// polygon covering the entire effective area.
fn from_geo_multipolygon(mp: &MultiPolygon<f64>) -> Vec<Point> {
    use geo::ConvexHull;
    let all_coords: Vec<geo_types::Coord<f64>> =
        mp.0.iter()
            .flat_map(|poly| poly.exterior().coords().cloned())
            .collect();
    if all_coords.is_empty() {
        return Vec::new();
    }
    let ls = geo_types::LineString::new(all_coords);
    let hull = geo_types::Polygon::new(ls, vec![]).convex_hull();
    hull.exterior()
        .coords()
        .map(|c| Point {
            x: c.x as f32,
            y: c.y as f32,
        })
        .collect()
}

/// Helper to buffer a polygon (positive distance expands, negative contracts)
fn buffer_poly(poly: &Polygon<f64>, distance: f32) -> MultiPolygon<f64> {
    buffer_polygon(poly, distance as f64)
}

/// Compute effective endzone: Original ∪ (Buffered ∩ Field)
/// Players in original zone always count + expanded area within field
pub fn compute_effective_endzone_polygon(
    ez_points: &[Point],
    field_points: &[Point],
    buffer_dist: f32,
) -> Vec<Point> {
    let ez_poly = to_geo_polygon(ez_points);
    let ez_buffered = buffer_poly(&ez_poly, buffer_dist);
    let field_poly = to_geo_polygon(field_points);
    let field_mp = MultiPolygon(vec![field_poly]);

    // Intersection: only buffer area within field
    let intersection = ez_buffered.intersection(&field_mp);

    // Union: original + buffered area within field
    let ez_mp = MultiPolygon(vec![ez_poly]);
    let effective = ez_mp.union(&intersection);

    from_geo_multipolygon(&effective)
}

/// Calculate buffer distance as % of polygon's own diagonal
pub fn compute_buffer_distance(points: &[Point], buffer_pct: f32) -> f32 {
    if points.is_empty() {
        return 0.0;
    }

    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for p in points {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let w = max_x - min_x;
    let h = max_y - min_y;
    let diag = (w.powi(2) + h.powi(2)).sqrt();
    diag * buffer_pct
}

/// Compute bbox with CROP padding (separate from feature buffer)
/// Crop padding ensures we capture edges after feature buffering
pub fn compute_bbox_with_crop_padding(points: &[Point], crop_padding_pct: f32) -> Option<BBox> {
    if points.is_empty() {
        return None;
    }

    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for p in points {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    // Add crop padding (separate from feature buffer)
    let x1 = (min_x - crop_padding_pct).max(0.0);
    let y1 = (min_y - crop_padding_pct).max(0.0);
    let x2 = (max_x + crop_padding_pct).min(1.0);
    let y2 = (max_y + crop_padding_pct).min(1.0);

    let w = (x2 - x1).max(0.0);
    let h = (y2 - y1).max(0.0);

    // Validate bbox has non-zero dimensions
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    Some(BBox { x: x1, y: y1, w, h })
}

/// Robust point-in-polygon using geo crate
#[allow(dead_code)]
pub fn is_point_in_polygon_robust(x: f32, y: f32, polygon: &[Point]) -> bool {
    let poly = to_geo_polygon(polygon);
    let point = GeoPoint::new(x as f64, y as f64);
    poly.contains(&point)
}

/// Transforms polygon coordinates from global to crop-local space.
pub fn transform_polygon(poly: &[Point], bbox: &BBox, crop_w: f32, crop_h: f32) -> Vec<Point> {
    poly.iter()
        .map(|p| {
            let px = ((p.x - bbox.x) / bbox.w) * crop_w;
            let py = ((p.y - bbox.y) / bbox.h) * crop_h;
            Point { x: px, y: py }
        })
        .collect()
}

/// Transform a detection from EZ crop pixel coordinates to overview pixel coordinates.
///
/// Two-step coordinate transform:
/// 1. EZ pixel → global normalized coordinates
/// 2. Global normalized → overview pixel coordinates
///
/// # Arguments
/// - `det`: the detection bbox in EZ crop pixel coordinates
/// - `ez_bbox`: the EZ crop's bounding box in normalized global coordinates
/// - `ez_w`, `ez_h`: EZ crop image dimensions in pixels
/// - `ov_bbox`: the overview crop's bounding box in normalized global coordinates
/// - `ov_w`, `ov_h`: overview crop image dimensions in pixels
pub fn transform_ez_to_overview(
    det: &PipelineBBox,
    ez_bbox: &PipelineBBox,
    ez_w: f32,
    ez_h: f32,
    ov_bbox: &PipelineBBox,
    ov_w: f32,
    ov_h: f32,
) -> PipelineBBox {
    tracing::trace!(
        "Transform: EZ det [{:.1},{:.1},{:.1},{:.1}] → EZ bbox [{:.3},{:.3},{:.3},{:.3}] ({:.0}x{:.0}) → OV bbox [{:.3},{:.3},{:.3},{:.3}] ({:.0}x{:.0})",
        det.x, det.y, det.w, det.h,
        ez_bbox.x, ez_bbox.y, ez_bbox.w, ez_bbox.h, ez_w, ez_h,
        ov_bbox.x, ov_bbox.y, ov_bbox.w, ov_bbox.h, ov_w, ov_h
    );

    // EZ pixel → global normalized
    let global_x = ez_bbox.x + (det.x / ez_w) * ez_bbox.w;
    let global_y = ez_bbox.y + (det.y / ez_h) * ez_bbox.h;
    let global_w = (det.w / ez_w) * ez_bbox.w;
    let global_h = (det.h / ez_h) * ez_bbox.h;

    tracing::trace!(
        "Transform: Global normalized: [{:.3},{:.3},{:.3},{:.3}]",
        global_x,
        global_y,
        global_w,
        global_h
    );

    // Global normalized → overview pixel
    let ov_x = ((global_x - ov_bbox.x) / ov_bbox.w) * ov_w;
    let ov_y = ((global_y - ov_bbox.y) / ov_bbox.h) * ov_h;
    let ov_det_w = (global_w / ov_bbox.w) * ov_w;
    let ov_det_h = (global_h / ov_bbox.h) * ov_h;

    tracing::trace!(
        "Transform: Overview pixel: [{:.1},{:.1},{:.1},{:.1}]",
        ov_x,
        ov_y,
        ov_det_w,
        ov_det_h
    );

    // Warn if the transformed bbox is outside the overview bounds
    if ov_x < -10.0
        || ov_y < -10.0
        || ov_x + ov_det_w > ov_w + 10.0
        || ov_y + ov_det_h > ov_h + 10.0
    {
        tracing::warn!(
            "Transform: Resulting bbox [{:.1},{:.1},{:.1},{:.1}] is outside overview bounds [0,0,{:.0},{:.0}]",
            ov_x, ov_y, ov_det_w, ov_det_h, ov_w, ov_h
        );
    }

    PipelineBBox {
        x: ov_x,
        y: ov_y,
        w: ov_det_w,
        h: ov_det_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_artifacts::{BBox, Point};

    #[test]
    fn test_transform_polygon() {
        let bbox = BBox {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        let crop_w = 400.0;
        let crop_h = 200.0;

        // Point at top-left of bbox (global) -> top-left of crop (0,0)
        let p1 = Point { x: 100.0, y: 100.0 };
        // Point at center of bbox (global) -> center of crop
        let p2 = Point { x: 200.0, y: 150.0 };
        // Point at bottom-right of bbox (global) -> bottom-right of crop
        let p3 = Point { x: 300.0, y: 200.0 };

        let poly = vec![p1, p2, p3];
        let transformed = transform_polygon(&poly, &bbox, crop_w, crop_h);

        assert_eq!(transformed.len(), 3);

        // p1 -> (0, 0)
        assert!((transformed[0].x - 0.0).abs() < 1e-6);
        assert!((transformed[0].y - 0.0).abs() < 1e-6);

        // p2 -> (200, 100)  (bbox w=200, point is at +100 (50%). Crop w=400, expected +200.
        //                    bbox h=100, point is at +50 (50%). Crop h=200, expected +100)
        assert!((transformed[1].x - 200.0).abs() < 1e-6);
        assert!((transformed[1].y - 100.0).abs() < 1e-6);

        // p3 -> (400, 200)
        assert!((transformed[2].x - 400.0).abs() < 1e-6);
        assert!((transformed[2].y - 200.0).abs() < 1e-6);
    }

    #[test]
    fn test_transform_ez_to_overview_identity() {
        // When EZ crop bbox == overview crop bbox, transform is identity
        let det = PipelineBBox {
            x: 100.0,
            y: 50.0,
            w: 40.0,
            h: 60.0,
        };
        let bbox = PipelineBBox {
            x: 0.1,
            y: 0.1,
            w: 0.8,
            h: 0.8,
        };
        let result = transform_ez_to_overview(&det, &bbox, 640.0, 480.0, &bbox, 640.0, 480.0);
        assert!((result.x - det.x).abs() < 0.01);
        assert!((result.y - det.y).abs() < 0.01);
        assert!((result.w - det.w).abs() < 0.01);
        assert!((result.h - det.h).abs() < 0.01);
    }

    #[test]
    fn test_transform_ez_to_overview_left_crop() {
        // Left EZ crop covers the left third of the overview
        // Overview: x=0.0, y=0.0, w=1.0, h=0.5 → 1920x960 pixels
        // Left EZ: x=0.0, y=0.0, w=0.33, h=0.5 → 640x960 pixels
        let ov_bbox = PipelineBBox {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 0.5,
        };
        let ez_bbox = PipelineBBox {
            x: 0.0,
            y: 0.0,
            w: 0.33,
            h: 0.5,
        };

        // Detection at center of EZ crop: (320, 480) in EZ pixels
        let det = PipelineBBox {
            x: 300.0,
            y: 460.0,
            w: 40.0,
            h: 40.0,
        };

        let result =
            transform_ez_to_overview(&det, &ez_bbox, 640.0, 960.0, &ov_bbox, 1920.0, 960.0);

        // (300/640)*0.33 = 0.1546875 global x → (0.1546875/1.0)*1920 = 297.0 overview px
        assert!((result.x - 297.0).abs() < 1.0, "x: {}", result.x);
        // y: (460/960)*0.5 = 0.2395833 global → (0.2395833/0.5)*960 = 460 overview px
        assert!((result.y - 460.0).abs() < 1.0, "y: {}", result.y);
        // w: (40/640)*0.33 = 0.020625 global → (0.020625/1.0)*1920 = 39.6 overview px
        assert!((result.w - 39.6).abs() < 1.0, "w: {}", result.w);
    }
}
