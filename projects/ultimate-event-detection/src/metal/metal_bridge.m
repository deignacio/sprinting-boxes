#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <dispatch/dispatch.h>

typedef struct {
    float min_drop;
    float min_prepoint_duration;
    float min_post_duration;
    float max_post_proba;
    float absolute_threshold;
    float min_gap;
    float smoothing_window;
} MetalDetectorParams;

// Initialize Metal device and return opaque handle
void* gpu_init(void) {
    @autoreleasepool {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device) {
            return NULL;
        }
        return (__bridge_retained void*)device;
    }
}

// Get command queue from device
void* gpu_get_command_queue(void* device_ptr) {
    if (!device_ptr) return NULL;

    @autoreleasepool {
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_ptr;
        id<MTLCommandQueue> queue = [device newCommandQueue];

        if (!queue) return NULL;
        return (__bridge_retained void*)queue;
    }
}

// Get compute pipeline from library data
void* gpu_get_pipeline(void* device_ptr, const uint8_t* lib_data, size_t lib_len) {
    if (!device_ptr || !lib_data || lib_len == 0) return NULL;

    @autoreleasepool {
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_ptr;

        // Create dispatch_data from buffer
        dispatch_data_t libData = dispatch_data_create(
            lib_data,
            lib_len,
            dispatch_get_main_queue(),
            ^{ /* no-op release */ }
        );

        NSError* error = nil;
        id<MTLLibrary> library = [device newLibraryWithData:libData error:&error];

        if (!library || error) {
            NSLog(@"Failed to load Metal library: %@", error);
            return NULL;
        }

        id<MTLFunction> function = [library newFunctionWithName:@"detect_cliffs"];
        if (!function) {
            NSLog(@"Failed to get detect_cliffs function");
            return NULL;
        }

        id<MTLComputePipelineState> pipeline = [device newComputePipelineStateWithFunction:function error:&error];
        if (!pipeline || error) {
            NSLog(@"Failed to create compute pipeline: %@", error);
            return NULL;
        }

        return (__bridge_retained void*)pipeline;
    }
}

// Detect cliffs on GPU
int gpu_detect_cliffs(
    void* device_ptr,
    void* command_queue_ptr,
    void* pipeline_ptr,
    const float* scores,
    uint32_t score_len,
    const MetalDetectorParams* params,
    uint32_t* output)
{
    if (!device_ptr || !command_queue_ptr || !pipeline_ptr || !scores || !output || score_len == 0) {
        return -1;
    }

    @autoreleasepool {
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_ptr;
        id<MTLCommandQueue> commandQueue = (__bridge id<MTLCommandQueue>)command_queue_ptr;
        id<MTLComputePipelineState> pipeline = (__bridge id<MTLComputePipelineState>)pipeline_ptr;

        // Create buffers
        id<MTLBuffer> scoresBuffer = [device newBufferWithBytes:(void*)scores
                                                         length:score_len * sizeof(float)
                                                        options:MTLResourceStorageModeShared];
        if (!scoresBuffer) return -1;

        uint32_t len = score_len;
        id<MTLBuffer> lenBuffer = [device newBufferWithBytes:&len
                                                      length:sizeof(uint32_t)
                                                     options:MTLResourceStorageModeShared];
        if (!lenBuffer) return -1;

        id<MTLBuffer> paramsBuffer = [device newBufferWithBytes:(void*)params
                                                         length:sizeof(MetalDetectorParams)
                                                        options:MTLResourceStorageModeShared];
        if (!paramsBuffer) return -1;

        id<MTLBuffer> outputBuffer = [device newBufferWithBytes:output
                                                         length:score_len * sizeof(uint32_t)
                                                        options:MTLResourceStorageModeShared];
        if (!outputBuffer) return -1;

        // Create command buffer and encoder
        id<MTLCommandBuffer> commandBuffer = [commandQueue commandBuffer];
        if (!commandBuffer) return -1;

        id<MTLComputeCommandEncoder> encoder = [commandBuffer computeCommandEncoder];
        if (!encoder) return -1;

        [encoder setComputePipelineState:pipeline];
        [encoder setBuffer:scoresBuffer offset:0 atIndex:0];
        [encoder setBuffer:lenBuffer offset:0 atIndex:1];
        [encoder setBuffer:paramsBuffer offset:0 atIndex:2];
        [encoder setBuffer:outputBuffer offset:0 atIndex:3];

        // Dispatch compute kernel
        MTLSize gridSize = MTLSizeMake(score_len, 1, 1);
        MTLSize threadGroupSize = MTLSizeMake(256, 1, 1);

        [encoder dispatchThreadgroups:gridSize threadsPerThreadgroup:threadGroupSize];
        [encoder endEncoding];

        [commandBuffer commit];
        [commandBuffer waitUntilCompleted];

        // Copy results back
        memcpy(output, [outputBuffer contents], score_len * sizeof(uint32_t));

        return 0;
    }
}

// Release device
void gpu_release_device(void* device_ptr) {
    if (device_ptr) {
        CFRelease((void*)device_ptr);
    }
}

// Release command queue
void gpu_release_command_queue(void* queue_ptr) {
    if (queue_ptr) {
        CFRelease((void*)queue_ptr);
    }
}

// Release pipeline
void gpu_release_pipeline(void* pipeline_ptr) {
    if (pipeline_ptr) {
        CFRelease((void*)pipeline_ptr);
    }
}
