//! Objective-C FFI bindings for CoreML helper functions.
//!
//! Provides safe Rust wrappers around Objective-C CoreML helpers that handle
//! protocol conversions and data extraction.
#![cfg(target_os = "macos")]
#![allow(dead_code)]

use anyhow::Result;
use objc2::rc::Retained;
use objc2_foundation::{NSString, NSDictionary};
use objc2_core_ml::{MLFeatureValue, MLMultiArray, MLModel};
use objc2_core_video::CVPixelBuffer;
use std::ffi::c_float;

// Struct for prediction data extraction result
#[repr(C)]
pub struct PredictionExtractionResult {
    pub confidence_data: *mut c_float,
    pub confidence_count: isize,
    pub coordinates_data: *mut c_float,
    pub coordinates_count: isize,
    pub success: bool,
}

// Core Foundation utilities
extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
}

// Foreign function interface to Objective-C helpers
extern "C" {
    /// Create feature provider from CVPixelBuffer (main entry point)
    /// Builds dictionary and wraps in MLDictionaryFeatureProvider in ObjC
    fn mlCreateFeatureProviderFromPixelBuffer(
        pixelBuffer: *const objc2_core_video::CVPixelBuffer,
    ) -> *mut objc2::runtime::AnyObject;

    /// Create MLDictionaryFeatureProvider from dictionary
    fn mlDictionaryToFeatureProvider(
        dict: *const NSDictionary<NSString, MLFeatureValue>,
    ) -> *mut objc2::runtime::AnyObject;

    /// Run model prediction
    fn mlGetPredictionOutput(
        model: *const MLModel,
        input: *const objc2::runtime::AnyObject,
    ) -> *mut objc2::runtime::AnyObject;

    /// Extract both outputs from prediction in a single autoreleasepool
    /// Drains all intermediate autoreleased objects while extracting
    fn mlExtractPredictionData(
        output: *const objc2::runtime::AnyObject,
    ) -> PredictionExtractionResult;

    /// Extract float array from MLMultiArray
    fn mlMultiArrayToFloatArray(
        multiArray: *const MLMultiArray,
        outCount: *mut isize,
    ) -> *mut c_float;

    /// Get MLMultiArray output by name
    fn mlGetMultiArrayOutput(
        features: *const objc2::runtime::AnyObject,
        name: *const NSString,
    ) -> *const MLMultiArray;

    /// Free allocated float array
    fn free(ptr: *mut std::ffi::c_void);

    /// Create CVPixelBuffer from raw BGRA image data
    fn mlCreatePixelBufferFromBGRAData(
        data: *const u8,
        width: usize,
        height: usize,
        bytesPerRow: usize,
    ) -> *mut objc2_core_video::CVPixelBuffer;
}

/// Create feature provider from CVPixelBuffer (main entry point)
/// Handles dictionary building and wrapping in ObjC, avoiding Rust type system issues
pub unsafe fn create_feature_provider_from_pixel_buffer(
    pixel_buffer: &CVPixelBuffer,
) -> Result<Retained<objc2::runtime::AnyObject>> {
    let provider_ptr = mlCreateFeatureProviderFromPixelBuffer(pixel_buffer);

    if provider_ptr.is_null() {
        anyhow::bail!("Failed to create feature provider from CVPixelBuffer");
    }

    Ok(Retained::from_raw(provider_ptr as *mut _)
        .ok_or_else(|| anyhow::anyhow!("Failed to create Retained from provider"))?)
}

/// Create feature provider from dictionary
pub unsafe fn create_feature_provider(
    dict: &NSDictionary<NSString, MLFeatureValue>,
) -> Result<Retained<objc2::runtime::AnyObject>> {
    let provider_ptr = mlDictionaryToFeatureProvider(dict);

    if provider_ptr.is_null() {
        anyhow::bail!("Failed to create MLDictionaryFeatureProvider");
    }

    Ok(Retained::from_raw(provider_ptr as *mut _)
        .ok_or_else(|| anyhow::anyhow!("Failed to create Retained from provider"))?)
}

/// Run model inference and immediately extract both outputs
/// The output and input objects are extracted and released within a local autoreleasepool
/// so they don't accumulate in the thread-level pool. Only the extracted Vec<f32> data
/// is returned to the caller.
///
/// This function only works on macOS; calling from other platforms will panic.
pub unsafe fn run_prediction_and_extract(
    model: &MLModel,
    input: &objc2::runtime::AnyObject,
) -> Result<(Vec<f32>, Vec<f32>)> {
    #[cfg(not(target_os = "macos"))]
    {
        unimplemented!("CoreML inference is only available on macOS")
    }

    // Wrap prediction and extraction in a local autoreleasepool so all ObjC objects
    // (output, input, MLMultiArray, etc.) drain immediately after extraction.
    #[cfg(target_os = "macos")]
    objc2::rc::autoreleasepool(|_| {
        let output_ptr = mlGetPredictionOutput(model, input);

        if output_ptr.is_null() {
            anyhow::bail!("Model prediction failed");
        }

        // Call ObjC function that does extraction
        let extraction_result = mlExtractPredictionData(output_ptr);

        if !extraction_result.success {
            anyhow::bail!("Failed to extract prediction data");
        }

        // Copy C float arrays into Rust Vecs and free C allocations
        let confidence_data = if extraction_result.confidence_count > 0 && !extraction_result.confidence_data.is_null() {
            let slice = std::slice::from_raw_parts(extraction_result.confidence_data, extraction_result.confidence_count as usize);
            slice.to_vec()
        } else {
            Vec::new()
        };
        if !extraction_result.confidence_data.is_null() {
            free(extraction_result.confidence_data as *mut std::ffi::c_void);
        }

        let coordinates_data = if extraction_result.coordinates_count > 0 && !extraction_result.coordinates_data.is_null() {
            let slice = std::slice::from_raw_parts(extraction_result.coordinates_data, extraction_result.coordinates_count as usize);
            slice.to_vec()
        } else {
            Vec::new()
        };
        if !extraction_result.coordinates_data.is_null() {
            free(extraction_result.coordinates_data as *mut std::ffi::c_void);
        }

        // output_ptr autoreleases and drains with this pool when it exits
        Ok((confidence_data, coordinates_data))
    })
}

/// Get MLMultiArray output from feature provider by name
pub unsafe fn get_multi_array_output(
    features: &objc2::runtime::AnyObject,
    name: &NSString,
) -> Result<Retained<MLMultiArray>> {
    let multi_array_ptr = mlGetMultiArrayOutput(features, name);

    if multi_array_ptr.is_null() {
        anyhow::bail!("Failed to get MLMultiArray output");
    }

    Ok(Retained::from_raw(multi_array_ptr as *mut _)
        .ok_or_else(|| anyhow::anyhow!("Failed to create Retained from MLMultiArray"))?)
}

/// Extract float array from MLMultiArray
pub unsafe fn multi_array_to_vec(multi_array: &MLMultiArray) -> Result<Vec<f32>> {
    let mut count: isize = 0;
    let ptr = mlMultiArrayToFloatArray(multi_array, &mut count);

    // Empty arrays are valid — return empty Vec
    if count <= 0 {
        if !ptr.is_null() {
            free(ptr as *mut std::ffi::c_void);
        }
        return Ok(Vec::new());
    }

    if ptr.is_null() {
        anyhow::bail!("Failed to extract float array from MLMultiArray");
    }

    // Safe copy into Rust-owned Vec, then explicitly free the C allocation
    // (Vec::from_raw_parts would use Rust's allocator to free a malloc pointer — UB)
    let slice = std::slice::from_raw_parts(ptr, count as usize);
    let vec = slice.to_vec();
    free(ptr as *mut std::ffi::c_void);
    Ok(vec)
}

/// Extract float data directly from prediction output by name
/// CRITICAL: This function extracts data WITHOUT holding Retained references to autoreleased objects.
/// This avoids the reference count mismatch where autoreleased (+0) objects are wrapped as Retained (+1).
/// Autoreleased objects are created locally and dropped immediately without being held.
pub unsafe fn extract_float_data_from_output(
    output: &objc2::runtime::AnyObject,
    name: &NSString,
) -> Result<Vec<f32>> {
    // Get the MLMultiArray output (autoreleased +0)
    let multi_array_ptr = mlGetMultiArrayOutput(output, name);

    if multi_array_ptr.is_null() {
        anyhow::bail!("Failed to get MLMultiArray output for '{:?}'", name);
    }

    // Extract float data immediately from the pointer
    let mut count: isize = 0;
    let float_ptr = mlMultiArrayToFloatArray(multi_array_ptr, &mut count);

    // The MLMultiArray (autoreleased) is now dropped without being wrapped in Retained
    // This prevents reference count mismatch

    if count <= 0 {
        if !float_ptr.is_null() {
            free(float_ptr as *mut std::ffi::c_void);
        }
        return Ok(Vec::new());
    }

    if float_ptr.is_null() {
        anyhow::bail!("Failed to extract float array from output");
    }

    // Copy into Rust-owned Vec and free C allocation
    let slice = std::slice::from_raw_parts(float_ptr, count as usize);
    let vec = slice.to_vec();
    free(float_ptr as *mut std::ffi::c_void);
    Ok(vec)
}

/// Create CVPixelBuffer from raw BGRA image data
/// Takes ownership of copying data into a new CVPixelBuffer
pub unsafe fn create_pixel_buffer_from_bgra_data(
    data: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Result<Retained<CVPixelBuffer>> {
    let pb_ptr = mlCreatePixelBufferFromBGRAData(
        data.as_ptr(),
        width,
        height,
        bytes_per_row,
    );

    if pb_ptr.is_null() {
        anyhow::bail!("Failed to create CVPixelBuffer from BGRA data");
    }

    Ok(Retained::from_raw(pb_ptr)
        .ok_or_else(|| anyhow::anyhow!("Failed to create Retained from CVPixelBuffer"))?)
}
