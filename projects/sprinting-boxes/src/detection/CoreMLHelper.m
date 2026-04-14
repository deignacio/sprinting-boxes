#import "CoreMLHelper.h"
#import <CoreML/CoreML.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

/// Create feature provider from CVPixelBuffer
/// This is the main entry point - builds dictionary in ObjC, eliminating type bridging issues
id<MLFeatureProvider>
mlCreateFeatureProviderFromPixelBuffer(CVPixelBufferRef pixelBuffer) {
    if (pixelBuffer == NULL) {
        NSLog(@"pixelBuffer is NULL");
        return nil;
    }

    // Create feature value from CVPixelBuffer (zero-copy)
    MLFeatureValue *imageFeature =
        [MLFeatureValue featureValueWithPixelBuffer:pixelBuffer];

    if (imageFeature == nil) {
        NSLog(@"Failed to create MLFeatureValue from CVPixelBuffer");
        return nil;
    }

    // Create dictionary with "image" key (D-FINE model expects this)
    NSDictionary<NSString *, MLFeatureValue *> *inputDict =
        @{@"image" : imageFeature};

    // Wrap in MLDictionaryFeatureProvider
    NSError *error = nil;
    MLDictionaryFeatureProvider *provider =
        [[MLDictionaryFeatureProvider alloc] initWithDictionary:inputDict
                                                         error:&error];

    if (error != nil) {
        NSLog(@"Failed to create MLDictionaryFeatureProvider: %@", error);
        return nil;
    }

    return provider;
}

/// Convert NSDictionary to MLDictionaryFeatureProvider
/// Handles the protocol conversion that's difficult in Rust's objc2 type system
MLDictionaryFeatureProvider *
mlDictionaryToFeatureProvider(NSDictionary<NSString *, MLFeatureValue *> *dict) {
    NSError *error = nil;
    MLDictionaryFeatureProvider *provider =
        [[MLDictionaryFeatureProvider alloc] initWithDictionary:dict error:&error];

    if (error != nil) {
        NSLog(@"Failed to create MLDictionaryFeatureProvider: %@", error);
        return nil;
    }

    return provider;
}

/// Run model prediction with proper error handling
/// Must catch ObjC exceptions to prevent them from propagating into Rust
id<MLFeatureProvider>
mlGetPredictionOutput(MLModel *model, id<MLFeatureProvider> input) {
    NSError *error = nil;
    id<MLFeatureProvider> output = nil;

    @try {
        output = [model predictionFromFeatures:input error:&error];
    } @catch (NSException *exception) {
        NSLog(@"CoreML prediction threw exception: %@", exception.reason);
        return nil;
    }

    if (error != nil) {
        NSLog(@"CoreML prediction failed: %@", error);
        return nil;
    }

    return output;
}

/// Extract both outputs from prediction
/// Returns float pointers that must be freed by the caller
typedef struct {
    float *confidence_data;
    intptr_t confidence_count;
    float *coordinates_data;
    intptr_t coordinates_count;
    bool success;
} PredictionExtractionResult;

PredictionExtractionResult
mlExtractPredictionData(id<MLFeatureProvider> output) {
    PredictionExtractionResult result = {NULL, 0, NULL, 0, false};

    // Extract confidence data
    MLMultiArray *conf_array = [output featureValueForName:@"confidence"].multiArrayValue;
    if (conf_array == nil) {
        NSLog(@"Failed to get confidence MLMultiArray");
        return result;
    }
    intptr_t conf_count = 0;
    result.confidence_data = mlMultiArrayToFloatArray(conf_array, (NSInteger *)&conf_count);
    result.confidence_count = conf_count;

    // Extract coordinates data
    MLMultiArray *coord_array = [output featureValueForName:@"coordinates"].multiArrayValue;
    if (coord_array == nil) {
        NSLog(@"Failed to get coordinates MLMultiArray");
        if (result.confidence_data != NULL) {
            free(result.confidence_data);
            result.confidence_data = NULL;
        }
        return result;
    }
    intptr_t coord_count = 0;
    result.coordinates_data = mlMultiArrayToFloatArray(coord_array, (NSInteger *)&coord_count);
    result.coordinates_count = coord_count;

    result.success = true;
    return result;
}

/// Extract float array from MLMultiArray
/// Uses getBytesWithHandler: to safely access ANE/GPU-resident arrays.
/// The handler is called synchronously with a CPU-accessible pointer after
/// any necessary ANE/GPU → system memory transfer. Available macOS 12.3+.
float *mlMultiArrayToFloatArray(MLMultiArray *multiArray,
                                NSInteger *outCount) {
    if (multiArray == nil) {
        *outCount = 0;
        return NULL;
    }

    NSInteger count = multiArray.count;
    *outCount = count;

    // Handle empty arrays gracefully
    if (count == 0) {
        return NULL;
    }

    // Allocate output buffer
    float *floatArray = (float *)malloc(count * sizeof(float));
    if (floatArray == NULL) {
        return NULL;
    }

    MLMultiArrayDataType dataType = multiArray.dataType;
    __block BOOL success = NO;

    // getBytesWithHandler: is the correct API for accessing ANE-resident arrays.
    // Note: one-argument block (bytes, size), no :error: parameter.
    @try {
        [multiArray getBytesWithHandler:^(const void *bytes, NSInteger size) {
            // Empty arrays (size=0) are valid — just mark success without copying
            if (size <= 0) {
                success = YES;
                return;
            }

            if (bytes == NULL) {
                return;
            }

            if (dataType == MLMultiArrayDataTypeFloat32) {
                // Direct copy
                if (count > 0) {
                    memcpy(floatArray, bytes, count * sizeof(float));
                }
            } else if (dataType == MLMultiArrayDataTypeDouble) {
                // Convert Float64 → Float32 (Apple YOLOv3 outputs Double)
                const double *src = (const double *)bytes;
                for (NSInteger i = 0; i < count; i++) {
                    floatArray[i] = (float)src[i];
                }
            } else if (dataType == MLMultiArrayDataTypeFloat16) {
                // Convert Float16 → Float32 (D-FINE outputs Float16 via Neural Engine)
                const __fp16 *src = (const __fp16 *)bytes;
                for (NSInteger i = 0; i < count; i++) {
                    floatArray[i] = (float)src[i];
                }
            } else if (dataType == MLMultiArrayDataTypeInt32) {
                // Convert Int32 → Float32
                const int32_t *src = (const int32_t *)bytes;
                for (NSInteger i = 0; i < count; i++) {
                    floatArray[i] = (float)src[i];
                }
            } else {
                NSLog(@"Unsupported MLMultiArray dataType: %ld", (long)dataType);
                return;
            }
            success = YES;
        }];
    } @catch (NSException *exception) {
        NSLog(@"Exception in getBytesWithHandler: %@", exception.reason);
        free(floatArray);
        *outCount = 0;
        return NULL;
    }

    if (!success) {
        free(floatArray);
        *outCount = 0;
        return NULL;
    }

    return floatArray;
}

/// Get MLMultiArray from feature provider by name
/// Extracts specific output tensors from the model's output
/// Note: featureValueForName: always returns MLFeatureValue, which wraps the actual data
MLMultiArray *
mlGetMultiArrayOutput(id<MLFeatureProvider> features, NSString *name) {
    if (features == nil || name == nil) {
        return nil;
    }

    @try {
        // featureValueForName: returns MLFeatureValue, not the raw type
        id feature = [features featureValueForName:name];

        if (feature == nil) {
            // Feature not found (expected for some output names we scan)
            return nil;
        }

        // Unwrap from MLFeatureValue
        if ([feature isKindOfClass:[MLFeatureValue class]]) {
            MLFeatureValue *fv = (MLFeatureValue *)feature;
            MLMultiArray *arr = fv.multiArrayValue;
            if (arr != nil) {
                return arr;
            } else {
                NSLog(@"Feature '%@': MLFeatureValue has no multiArrayValue (may be empty or different type)", name);
                return nil;
            }
        }

        NSLog(@"Feature '%@' is unexpected type: %@", name, [feature class]);
        return nil;
    } @catch (NSException *exception) {
        NSLog(@"Exception accessing feature '%@': %@", name, exception.reason);
        return nil;
    }
}

/// Create CVPixelBuffer from raw BGRA image data
/// Converts BGR→RGB and creates an RGB pixel buffer for CoreML compatibility
CVPixelBufferRef
mlCreatePixelBufferFromBGRAData(const uint8_t *data,
                                 size_t width,
                                 size_t height,
                                 size_t bytesPerRow) {
    if (data == NULL || width == 0 || height == 0) {
        NSLog(@"Invalid parameters: data=%p, width=%zu, height=%zu", data, width, height);
        return NULL;
    }

    CVPixelBufferRef pixelBuffer = NULL;
    // Create BGRA pixel buffer - CoreML can handle BGRA or we'll handle color conversion in feature provider
    CVReturn status = CVPixelBufferCreate(
        kCFAllocatorDefault,
        width,
        height,
        kCVPixelFormatType_32BGRA,
        NULL, // attributes (default)
        &pixelBuffer);

    if (status != kCVReturnSuccess) {
        NSLog(@"ERROR: CVPixelBufferCreate failed with status=%d", status);
        return NULL;
    }

    if (pixelBuffer == NULL) {
        NSLog(@"CVPixelBufferCreate returned NULL");
        return NULL;
    }

    // Lock buffer and copy data
    CVPixelBufferLockBaseAddress(pixelBuffer, 0);

    uint8_t *baseAddress = CVPixelBufferGetBaseAddress(pixelBuffer);
    size_t bufferBytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);

    // Copy pixel data row by row (BGRA format)
    for (size_t y = 0; y < height; y++) {
        if (baseAddress == NULL) {
            NSLog(@"ERROR: baseAddress became NULL during copy");
            CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
            CVPixelBufferRelease(pixelBuffer);
            return NULL;
        }

        const uint8_t *srcRow = data + (y * bytesPerRow);
        uint8_t *dstRow = baseAddress + (y * bufferBytesPerRow);

        // Safety check: ensure we're not writing past buffer bounds
        if (dstRow == NULL) {
            NSLog(@"ERROR: dstRow is NULL at y=%zu", y);
            CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
            CVPixelBufferRelease(pixelBuffer);
            return NULL;
        }

        memcpy(dstRow, srcRow, width * 4); // 4 bytes per BGRA pixel
    }

    CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);

    return pixelBuffer;
}
