#include <metal_stdlib>
using namespace metal;

// Detector configuration packed into a struct
struct DetectorParams {
    float min_drop;
    float min_prepoint_duration;
    float min_post_duration;
    float max_post_proba;
    float absolute_threshold;
    float min_gap;
    float smoothing_window;
};

// Compute median of a small sorted array (up to 30 elements)
inline float compute_median(const thread float* values, uint count) {
    if (count == 0) return 0.0;
    // Note: assumes values are already sorted
    return values[count / 2];
}

// Compute moving average for smoothing
inline float compute_smoothed_at(const device float* scores, uint len, uint i, uint window) {
    uint start = (i >= window - 1) ? (i - window + 1) : 0;
    uint end = i + 1;

    float sum = 0.0;
    for (uint j = start; j < end; j++) {
        sum += scores[j];
    }
    return sum / (float)(end - start);
}

// Sort a small array in-place (for median calculation)
inline void sort_small_array(thread float* arr, uint count) {
    // Simple bubble sort for small arrays
    for (uint i = 0; i < count; i++) {
        for (uint j = i + 1; j < count; j++) {
            if (arr[j] < arr[i]) {
                float tmp = arr[i];
                arr[i] = arr[j];
                arr[j] = tmp;
            }
        }
    }
}

// Main kernel: check if frame at index i is a cliff
kernel void detect_cliffs(
    device const float* scores [[buffer(0)]],           // input scores
    device const uint* score_len [[buffer(1)]],         // length of scores
    device const DetectorParams* params [[buffer(2)]],  // config
    device uint* detected [[buffer(3)]],                // output: 1 if cliff, 0 otherwise
    uint gid [[thread_position_in_grid]])
{
    uint len = score_len[0];
    if (gid >= len) return;

    DetectorParams cfg = params[0];
    uint i = gid;
    uint min_pre = (uint)cfg.min_prepoint_duration;
    uint min_post = (uint)cfg.min_post_duration;
    uint window = (uint)cfg.smoothing_window;

    // Bounds check
    if (len < min_pre + min_post) {
        detected[i] = 0;
        return;
    }
    if (i < min_pre || i + min_post >= len) {
        detected[i] = 0;
        return;
    }

    // Compute smoothed values at i and i+1
    float smoothed_i = compute_smoothed_at(scores, len, i, window);
    float smoothed_i_next = compute_smoothed_at(scores, len, i + 1, window);

    // Check drop
    float drop = smoothed_i - smoothed_i_next;
    uint start_w = (i >= window - 1) ? (i - window + 1) : 0;
    float cumulative_drop = compute_smoothed_at(scores, len, start_w, window) - smoothed_i_next;
    float effective_drop = max(drop, cumulative_drop);

    if (effective_drop < cfg.min_drop) {
        detected[i] = 0;
        return;
    }

    if (smoothed_i_next > cfg.absolute_threshold) {
        detected[i] = 0;
        return;
    }

    // Check pre-point plateau (median of pre window >= 0.5)
    uint start_pre = (i >= min_pre) ? (i - min_pre) : 0;
    uint pre_len = i - start_pre;
    if (pre_len < min_pre) {
        detected[i] = 0;
        return;
    }

    // Compute median of pre-window (load into local memory, sort, find median)
    // Use smoothed scores for pre-window (like Python does)
    float pre_values[30];
    for (uint j = 0; j < pre_len && j < 30; j++) {
        pre_values[j] = compute_smoothed_at(scores, len, start_pre + j, window);
    }
    sort_small_array(pre_values, pre_len);
    float median_pre = pre_values[pre_len / 2];

    if (median_pre < 0.5) {
        detected[i] = 0;
        return;
    }

    // Check post-point stability (median of post window <= max_post_proba)
    uint post_start = i + 1;
    uint post_end = min(post_start + min_post, len);
    uint post_len = post_end - post_start;

    if (post_len < min_post) {
        detected[i] = 0;
        return;
    }

    float post_values[30];
    for (uint j = 0; j < post_len && j < 30; j++) {
        post_values[j] = scores[post_start + j];
    }
    sort_small_array(post_values, post_len);
    float median_post = post_values[post_len / 2];

    if (median_post > cfg.max_post_proba) {
        detected[i] = 0;
        return;
    }

    detected[i] = 1;
}
