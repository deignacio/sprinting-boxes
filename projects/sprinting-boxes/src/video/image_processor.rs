//! Image frame processing for converting raw images to preprocessed frames.
//!
//! Handles the transformation of raw image data into pipeline-ready frames,
//! including polygon coordinate transforms and regional polygon calculations.

use anyhow::Result;
use crate::pipeline::types::{CropConfig, CropData, PreprocessedFrame};
use opencv::core::{Mat, MatTraitConst};

/// Processes a raw image into a PreprocessedFrame with proper polygon transforms.
///
/// Takes an image Mat and applies the overview crop configuration to generate
/// a PreprocessedFrame with transformed polygon coordinates.
pub fn process_image_to_frame(
    unit_id: usize,
    mat: Mat,
    overview_config: &CropConfig,
) -> Result<PreprocessedFrame> {
    let crop_w = mat.cols() as f32;
    let crop_h = mat.rows() as f32;

    let original_poly_local = crate::geometry::transform_polygon(
        &overview_config.original_polygon,
        &overview_config.bbox,
        crop_w,
        crop_h,
    );
    let effective_poly_local = crate::geometry::transform_polygon(
        &overview_config.effective_polygon,
        &overview_config.bbox,
        crop_w,
        crop_h,
    );
    let regions_local = overview_config
        .regions
        .iter()
        .map(|r| crate::pipeline::types::RegionalPolygon {
            name: r.name.clone(),
            polygon: crate::geometry::transform_polygon(
                &r.polygon,
                &overview_config.bbox,
                crop_w,
                crop_h,
            ),
            effective_polygon: crate::geometry::transform_polygon(
                &r.effective_polygon,
                &overview_config.bbox,
                crop_w,
                crop_h,
            ),
        })
        .collect();

    let crop_data = CropData {
        image: mat,
        original_polygon: original_poly_local,
        effective_polygon: effective_poly_local,
        suffix: overview_config.suffix.clone(),
        regions: regions_local,
        source_bbox: overview_config.bbox,
    };

    Ok(PreprocessedFrame {
        id: unit_id,
        crops: vec![crop_data],
    })
}

/// Processes an empty frame (due to read error) with dummy dimensions.
///
/// Creates a PreprocessedFrame with an empty Mat when image reading fails,
/// preserving frame sequence integrity in the pipeline.
pub fn process_empty_frame(
    unit_id: usize,
    overview_config: &CropConfig,
) -> Result<PreprocessedFrame> {
    let crop_w = 1.0; // dummy dimensions for empty frame
    let crop_h = 1.0;

    let regions_local = overview_config
        .regions
        .iter()
        .map(|r| crate::pipeline::types::RegionalPolygon {
            name: r.name.clone(),
            polygon: crate::geometry::transform_polygon(
                &r.polygon,
                &overview_config.bbox,
                crop_w,
                crop_h,
            ),
            effective_polygon: crate::geometry::transform_polygon(
                &r.effective_polygon,
                &overview_config.bbox,
                crop_w,
                crop_h,
            ),
        })
        .collect();

    let crop_data = CropData {
        image: opencv::core::Mat::default(),
        original_polygon: overview_config.original_polygon.clone(),
        effective_polygon: overview_config.effective_polygon.clone(),
        suffix: overview_config.suffix.clone(),
        regions: regions_local,
        source_bbox: overview_config.bbox,
    };

    Ok(PreprocessedFrame {
        id: unit_id,
        crops: vec![crop_data],
    })
}
