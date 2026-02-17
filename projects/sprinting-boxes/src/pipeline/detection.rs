use anyhow::{anyhow, Result};
use image::{DynamicImage, ImageBuffer, Rgb};
use opencv::core::Mat;
use opencv::prelude::*;
use usls::models::RTDETR;
use usls::Config;

/// A wrapper around the USLS RT-DETR model that handles BGR-to-RGB conversion
/// and corrects for aspect-ratio padding bugs in the underlying model library.
pub struct ObjectDetector {
    model: RTDETR,
}

impl ObjectDetector {
    /// Create a new detector with the given model path.
    pub fn new(model_path: &str) -> Result<Self> {
        let config = Config::default()
            .with_model_file(model_path)
            .with_class_names(&usls::NAMES_COCO_80);

        #[cfg(target_os = "macos")]
        let config = config.with_model_device(usls::Device::CoreMl);

        let config = config.commit()?;
        let model = RTDETR::new(config)?;
        Ok(Self { model })
    }

    /// Run detection on an OpenCV Mat.
    pub fn detect(&mut self, image: &Mat) -> Result<Vec<usls::Hbb>> {
        let results = self.detect_batch(std::slice::from_ref(image))?;
        Ok(results.into_iter().next().unwrap_or_default())
    }

    /// Run detection on a batch of OpenCV Mats.
    pub fn detect_batch(&mut self, images: &[Mat]) -> Result<Vec<Vec<usls::Hbb>>> {
        const CHUNK_SIZE: usize = 8;
        let mut final_batch_results = Vec::with_capacity(images.len());

        for chunk in images.chunks(CHUNK_SIZE) {
            let chunk_start = std::time::Instant::now();
            let mut usls_images = Vec::with_capacity(chunk.len());
            let mut corrections = Vec::with_capacity(chunk.len());

            for image in chunk {
                let dynamic_image = mat_to_dynamic_image(image)?;

                // Correction calculations (USLS RT-DETR bug workaround)
                let size = image.size()?;
                let img_w = size.width as f32;
                let img_h = size.height as f32;

                let (x_corr, y_corr) = if img_w > img_h {
                    (img_w / img_h, 1.0)
                } else if img_h > img_w {
                    (1.0, img_h / img_w)
                } else {
                    (1.0, 1.0)
                };
                corrections.push((x_corr, y_corr));

                usls_images.push(usls::Image::from(dynamic_image));
            }

            let preprocess_duration = chunk_start.elapsed();
            let forward_start = std::time::Instant::now();
            let results = self.model.forward(&usls_images)?;
            let forward_duration = forward_start.elapsed();

            tracing::debug!(
                "Batch chunk (size {}): Preprocess: {:?}, Inference: {:?}",
                chunk.len(),
                preprocess_duration,
                forward_duration
            );

            for (y, (x_correction, y_correction)) in results.into_iter().zip(corrections) {
                let corrected_hbbs: Vec<usls::Hbb> = y
                    .hbbs
                    .into_iter()
                    .map(|hbb| {
                        let x = hbb.xmin() * x_correction;
                        let w = hbb.width() * x_correction;
                        let y_coord = hbb.ymin() * y_correction;
                        let h = hbb.height() * y_correction;

                        let mut new_hbb =
                            usls::Hbb::default().with_xyxy(x, y_coord, x + w, y_coord + h);

                        if let Some(conf) = hbb.confidence() {
                            new_hbb = new_hbb.with_confidence(conf);
                        }
                        if let Some(id) = hbb.id() {
                            new_hbb = new_hbb.with_id(id);
                        }
                        if let Some(name) = hbb.name() {
                            new_hbb = new_hbb.with_name(name);
                        }

                        new_hbb
                    })
                    .collect();
                final_batch_results.push(corrected_hbbs);
            }
        }

        Ok(final_batch_results)
    }
}

/// Convert an OpenCV Mat (BGR) to an image::DynamicImage (RGB)
fn mat_to_dynamic_image(mat: &Mat) -> Result<DynamicImage> {
    let mut rgb_mat = Mat::default();
    opencv::imgproc::cvt_color_def(mat, &mut rgb_mat, opencv::imgproc::COLOR_BGR2RGB)?;

    let size = rgb_mat.size()?;
    let width = size.width as u32;
    let height = size.height as u32;

    if !rgb_mat.is_continuous() {
        return Err(anyhow!("Mat is not continuous"));
    }

    let data_bytes = rgb_mat.data_bytes()?;
    let buffer = data_bytes.to_vec();

    let img_buffer = ImageBuffer::<Rgb<u8>, _>::from_vec(width, height, buffer)
        .ok_or_else(|| anyhow!("Failed to create ImageBuffer from Mat data"))?;

    Ok(DynamicImage::ImageRgb8(img_buffer))
}
