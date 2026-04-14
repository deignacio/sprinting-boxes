#ifndef COREMLHELPER_H
#define COREMLHELPER_H

#import <CoreML/CoreML.h>
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// Create feature provider from CVPixelBuffer
/// Builds the input dictionary and wraps it in MLDictionaryFeatureProvider
/// @param pixelBuffer The input image as CVPixelBuffer (zero-copy)
/// @return MLDictionaryFeatureProvider ready for inference, or NULL on error
/// Caller owns the returned reference (+1 refcount via NS_RETURNS_RETAINED)
NS_RETURNS_RETAINED id<MLFeatureProvider> _Nullable
mlCreateFeatureProviderFromPixelBuffer(CVPixelBufferRef _Nonnull pixelBuffer);

/// Convert NSDictionary of feature values to MLFeatureProvider protocol object
/// This bridges the gap between Swift/Rust type systems and Objective-C protocols
/// @param dict Dictionary mapping input names to MLFeatureValue objects
/// @return MLDictionaryFeatureProvider (conforms to MLFeatureProvider)
MLDictionaryFeatureProvider * _Nonnull
mlDictionaryToFeatureProvider(NSDictionary<NSString *, MLFeatureValue *> * _Nonnull dict);

/// Run model prediction with feature provider input
/// @param model The MLModel to run inference on
/// @param input Feature provider containing input tensors
/// @return Feature provider containing output tensors, or NULL on error
id<MLFeatureProvider> _Nullable
mlGetPredictionOutput(MLModel * _Nonnull model,
                      id<MLFeatureProvider> _Nonnull input);

/// Extract float array from MLMultiArray output
/// @param multiArray The output tensor from model
/// @param outCount Pointer to store array length (number of floats)
/// @return Pointer to float array (caller must free), or NULL on error
float * _Nullable mlMultiArrayToFloatArray(MLMultiArray * _Nonnull multiArray,
                                            NSInteger * _Nonnull outCount);

/// Get MLMultiArray from feature provider by name
/// @param features The output feature provider from inference
/// @param name The output tensor name (e.g., "confidence", "coordinates")
/// @return MLMultiArray for the named output, or NULL if not found
MLMultiArray * _Nullable
mlGetMultiArrayOutput(id<MLFeatureProvider> _Nonnull features,
                      NSString * _Nonnull name);

/// Create CVPixelBuffer from raw BGRA image data
/// @param data Pointer to raw BGRA pixel data
/// @param width Image width in pixels
/// @param height Image height in pixels
/// @param bytesPerRow Bytes per row (stride), accounting for alignment
/// @return CVPixelBuffer ready for CoreML input, or NULL on error
CVPixelBufferRef _Nullable
mlCreatePixelBufferFromBGRAData(const uint8_t * _Nonnull data,
                                 size_t width,
                                 size_t height,
                                 size_t bytesPerRow);

NS_ASSUME_NONNULL_END

#endif /* COREMLHELPER_H */
