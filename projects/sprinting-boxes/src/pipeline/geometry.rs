use crate::pipeline::types::{BBox, Point};
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

/// Convert geo_types MultiPolygon back to our Points
fn from_geo_multipolygon(mp: &MultiPolygon<f64>) -> Vec<Point> {
    mp.0.iter()
        .flat_map(|poly| {
            poly.exterior().coords().map(|c| Point {
                x: c.x as f32,
                y: c.y as f32,
            })
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
