/* Independent Wayland/shared-memory input fixture, not linked to flatpak-rs. */
#define _GNU_SOURCE
#include <assert.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *wm;
static struct wl_surface *surface;
static struct wl_seat *seat;
static int inputs;

static void release(void *data, struct wl_buffer *buffer) {
    munmap(data, 320 * 240 * 4);
    wl_buffer_destroy(buffer);
}
static const struct wl_buffer_listener buffer_listener = { release };
static void draw(void) {
    int fd = memfd_create("fixture-pixels", MFD_CLOEXEC);
    assert(fd >= 0 && ftruncate(fd, 320 * 240 * 4) == 0);
    uint32_t *pixels = mmap(NULL, 320 * 240 * 4, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    assert(pixels != MAP_FAILED);
    for (int i = 0; i < 320 * 240; i++) pixels[i] = inputs == 3 ? 0xff00ff00 : 0xffff0000;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, 320 * 240 * 4);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(pool, 0, 320, 240, 320 * 4, WL_SHM_FORMAT_ARGB8888);
    wl_buffer_add_listener(buffer, &buffer_listener, pixels);
    wl_shm_pool_destroy(pool);
    close(fd);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage(surface, 0, 0, 320, 240);
    wl_surface_commit(surface);
    if (inputs == 3) { puts("keyboard-and-pointer"); fflush(stdout); }
}
static void ping(void *data, struct xdg_wm_base *base, uint32_t serial) { (void)data; xdg_wm_base_pong(base, serial); }
static const struct xdg_wm_base_listener wm_listener = { ping };
static void configure(void *data, struct xdg_surface *s, uint32_t serial) { (void)data; xdg_surface_ack_configure(s, serial); draw(); }
static const struct xdg_surface_listener surface_listener = { configure };
static void keymap(void *d, struct wl_keyboard *k, uint32_t f, int fd, uint32_t size) { (void)d; (void)k; (void)f; (void)size; close(fd); }
static void key_enter(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s, struct wl_array *keys) { (void)d; (void)k; (void)serial; (void)s; (void)keys; }
static void key_leave(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s) { (void)d; (void)k; (void)serial; (void)s; }
static void key(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t time, uint32_t code, uint32_t state) { (void)d; (void)k; (void)serial; (void)time; (void)code; if (state) { inputs |= 1; draw(); } }
static void modifiers(void *d, struct wl_keyboard *k, uint32_t s, uint32_t a, uint32_t b, uint32_t c, uint32_t g) { (void)d; (void)k; (void)s; (void)a; (void)b; (void)c; (void)g; }
static const struct wl_keyboard_listener keyboard_listener = { .keymap=keymap, .enter=key_enter, .leave=key_leave, .key=key, .modifiers=modifiers };
static void enter(void *d, struct wl_pointer *p, uint32_t s, struct wl_surface *w, wl_fixed_t x, wl_fixed_t y) { (void)d; (void)p; (void)s; (void)w; (void)x; (void)y; }
static void leave(void *d, struct wl_pointer *p, uint32_t s, struct wl_surface *w) { (void)d; (void)p; (void)s; (void)w; }
static void motion(void *d, struct wl_pointer *p, uint32_t t, wl_fixed_t x, wl_fixed_t y) { (void)d; (void)p; (void)t; (void)x; (void)y; }
static void button(void *d, struct wl_pointer *p, uint32_t s, uint32_t t, uint32_t b, uint32_t state) { (void)d; (void)p; (void)s; (void)t; (void)b; if (state) { inputs |= 2; draw(); } }
static void axis(void *d, struct wl_pointer *p, uint32_t t, uint32_t a, wl_fixed_t v) { (void)d; (void)p; (void)t; (void)a; (void)v; }
static const struct wl_pointer_listener pointer_listener = { .enter=enter, .leave=leave, .motion=motion, .button=button, .axis=axis };
static void capabilities(void *d, struct wl_seat *s, uint32_t caps) {
    (void)d;
    if (caps & WL_SEAT_CAPABILITY_KEYBOARD) wl_keyboard_add_listener(wl_seat_get_keyboard(s), &keyboard_listener, NULL);
    if (caps & WL_SEAT_CAPABILITY_POINTER) wl_pointer_add_listener(wl_seat_get_pointer(s), &pointer_listener, NULL);
}
static const struct wl_seat_listener seat_listener = { .capabilities=capabilities };
static void global(void *d, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)d; (void)version;
    if (!strcmp(interface, "wl_compositor")) compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 1);
    if (!strcmp(interface, "wl_shm")) shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    if (!strcmp(interface, "xdg_wm_base")) { wm = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1); xdg_wm_base_add_listener(wm, &wm_listener, NULL); }
    if (!strcmp(interface, "wl_seat")) { seat = wl_registry_bind(registry, name, &wl_seat_interface, 1); wl_seat_add_listener(seat, &seat_listener, NULL); }
}
static void removed(void *d, struct wl_registry *r, uint32_t name) { (void)d; (void)r; (void)name; }
static const struct wl_registry_listener registry_listener = { global, removed };
int main(void) {
    assert(access("/home/alice", F_OK) < 0);
    assert(access("/home/razze", F_OK) < 0);
    assert(access("/run/user/1000/flatpak-rs/management.sock", F_OK) < 0);
    assert(access("/run/user/0/flatpak-rs/management.sock", F_OK) < 0);
    assert(getenv("DISPLAY") == NULL);
    assert(getenv("DBUS_SESSION_BUS_ADDRESS") == NULL);
    for (int fd = 3; fd < 64; fd++) assert(fcntl(fd, F_GETFD) < 0);
    assert(access("/run/user/1000/bus", F_OK) < 0);
    assert(access("/run/user/0/bus", F_OK) < 0);
    assert(access("/dev/dri", F_OK) < 0);
    assert(access("/tmp/.X11-unix", F_OK) < 0);
    assert(socket(AF_INET, SOCK_STREAM, 0) < 0);
    assert(socket(AF_INET6, SOCK_STREAM, 0) < 0);
    display = wl_display_connect(NULL);
    assert(display);
    wl_registry_add_listener(wl_display_get_registry(display), &registry_listener, NULL);
    assert(wl_display_roundtrip(display) >= 0 && compositor && shm && wm);
    surface = wl_compositor_create_surface(compositor);
    struct xdg_surface *xdg = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdg, &surface_listener, NULL);
    struct xdg_toplevel *top = xdg_surface_get_toplevel(xdg);
    xdg_toplevel_set_title(top, "flatpak-rs graphical fixture");
    xdg_toplevel_set_app_id(top, "org.example.Graphical");
    wl_surface_commit(surface);
    while (wl_display_dispatch(display) >= 0) {}
    return 0;
}
