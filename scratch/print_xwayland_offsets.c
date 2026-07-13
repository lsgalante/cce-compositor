#define WLR_USE_UNSTABLE
#include <wlr/xwayland.h>
#include <stdio.h>
#include <stddef.h>

#define PRINT_OFFSET(struct_name, member) \
    printf("offset of " #member ": %zu\n", offsetof(struct_name, member))

int main() {
    printf("C sizeof(struct wlr_xwayland): %zu\n", sizeof(struct wlr_xwayland));
    PRINT_OFFSET(struct wlr_xwayland, server);
    PRINT_OFFSET(struct wlr_xwayland, own_server);
    PRINT_OFFSET(struct wlr_xwayland, xwm);
    PRINT_OFFSET(struct wlr_xwayland, shell_v1);
    PRINT_OFFSET(struct wlr_xwayland, display_name);
    PRINT_OFFSET(struct wlr_xwayland, wl_display);
    PRINT_OFFSET(struct wlr_xwayland, compositor);
    PRINT_OFFSET(struct wlr_xwayland, seat);
    PRINT_OFFSET(struct wlr_xwayland, events);
    return 0;
}
