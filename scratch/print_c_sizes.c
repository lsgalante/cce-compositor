#define WLR_USE_UNSTABLE
#include <wlr/types/wlr_output.h>
#include <pixman.h>
#include <stdio.h>
#include <stddef.h>

int main() {
    printf("C sizeof(struct wlr_output_state): %zu\n", sizeof(struct wlr_output_state));
    printf("C alignof(struct wlr_output_state): %zu\n", _Alignof(struct wlr_output_state));
    printf("C sizeof(pixman_region32_t): %zu\n", sizeof(pixman_region32_t));
    printf("C alignof(pixman_region32_t): %zu\n", _Alignof(pixman_region32_t));

    printf("committed: %zu\n", offsetof(struct wlr_output_state, committed));
    printf("allow_reconfiguration: %zu\n", offsetof(struct wlr_output_state, allow_reconfiguration));
    printf("damage: %zu\n", offsetof(struct wlr_output_state, damage));
    printf("enabled: %zu\n", offsetof(struct wlr_output_state, enabled));
    printf("scale: %zu\n", offsetof(struct wlr_output_state, scale));
    printf("transform: %zu\n", offsetof(struct wlr_output_state, transform));
    printf("adaptive_sync_enabled: %zu\n", offsetof(struct wlr_output_state, adaptive_sync_enabled));
    printf("render_format: %zu\n", offsetof(struct wlr_output_state, render_format));
    printf("subpixel: %zu\n", offsetof(struct wlr_output_state, subpixel));
    printf("buffer: %zu\n", offsetof(struct wlr_output_state, buffer));
    printf("buffer_src_box: %zu\n", offsetof(struct wlr_output_state, buffer_src_box));
    printf("buffer_dst_box: %zu\n", offsetof(struct wlr_output_state, buffer_dst_box));
    printf("tearing_page_flip: %zu\n", offsetof(struct wlr_output_state, tearing_page_flip));
    printf("mode_type: %zu\n", offsetof(struct wlr_output_state, mode_type));
    printf("mode: %zu\n", offsetof(struct wlr_output_state, mode));
    printf("custom_mode: %zu\n", offsetof(struct wlr_output_state, custom_mode));
    printf("gamma_lut: %zu\n", offsetof(struct wlr_output_state, gamma_lut));
    printf("gamma_lut_size: %zu\n", offsetof(struct wlr_output_state, gamma_lut_size));
    printf("layers: %zu\n", offsetof(struct wlr_output_state, layers));
    printf("layers_len: %zu\n", offsetof(struct wlr_output_state, layers_len));
    printf("wait_timeline: %zu\n", offsetof(struct wlr_output_state, wait_timeline));
    printf("wait_point: %zu\n", offsetof(struct wlr_output_state, wait_point));
    printf("signal_timeline: %zu\n", offsetof(struct wlr_output_state, signal_timeline));
    printf("signal_point: %zu\n", offsetof(struct wlr_output_state, signal_point));
    return 0;
}
