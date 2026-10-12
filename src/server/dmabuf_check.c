// SPDX-License-Identifier: GPL-3.0-only
//
// The linux-dmabuf import check (wlr_linux_dmabuf_v1_set_check_dmabuf_callback).
//
// wlroots' default check only asks whether the renderer's device can import
// a client's DMA-BUF (a PRIME handle). That is not whether it can USE it: a
// buffer another GPU exported from memory this one cannot reach (an NVIDIA
// export from video memory, which GE-Proton's winewayland forwards for a
// window DXVK rendered on the dGPU) imports fine, and the first draw that
// samples it makes the kernel refuse the renderer's command batch. Mesa's
// iris driver answers that with abort() ("intel: the execbuf ioctl keeps
// returning ENOMEM"), so one client took the whole session down
// (2026-10-12).
//
// So on i915 a buffer some other driver exported is probed before it is
// accepted: a do-nothing batch that references it is submitted on a device
// fd of our own, which makes the kernel pin the buffer's pages exactly as
// the renderer's draw would, and returns the failure as an errno instead of
// an abort. A refused buffer fails the import, which the protocol reports to
// the client. Buffers i915 exported itself (nearly every client here) skip
// the probe: they are this GPU's own memory.

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <xf86drm.h>
#include <i915_drm.h>

#include <wlr/render/dmabuf.h>
#include <wlr/types/wlr_linux_dmabuf_v1.h>
#include <wlr/util/log.h>

#define MI_BATCH_BUFFER_END (0x0a << 23)

struct cce_dmabuf_check {
	// Our own open of the renderer's device. Never a dup of the renderer's
	// fd: GEM handles belong to the open file description, so a dup would
	// share Mesa's handle for the same buffer, and closing the probe's
	// handle would close the renderer's.
	int fd;
	bool i915;
	uint32_t batch; // a 4 KiB BO holding MI_BATCH_BUFFER_END, 0 if unusable
};

static struct cce_dmabuf_check *check_state;

// The name of the driver that exported a DMA-BUF ("i915", "nvidia", ...),
// from /proc/self/fdinfo. Empty when it cannot be read.
static void exporter_name(int fd, char *out, size_t out_len) {
	out[0] = '\0';
	char path[64];
	snprintf(path, sizeof(path), "/proc/self/fdinfo/%d", fd);
	FILE *f = fopen(path, "re");
	if (f == NULL) {
		return;
	}
	char line[256];
	while (fgets(line, sizeof(line), f) != NULL) {
		if (strncmp(line, "exp_name:", 9) == 0) {
			const char *v = line + 9;
			while (*v == ' ' || *v == '\t') {
				v++;
			}
			size_t n = strcspn(v, "\r\n");
			if (n >= out_len) {
				n = out_len - 1;
			}
			memcpy(out, v, n);
			out[n] = '\0';
			break;
		}
	}
	fclose(f);
}

static void gem_close(int fd, uint32_t handle) {
	struct drm_gem_close arg = { .handle = handle };
	drmIoctl(fd, DRM_IOCTL_GEM_CLOSE, &arg);
}

static uint32_t create_batch(int fd) {
	struct drm_i915_gem_create create = { .size = 4096 };
	if (drmIoctl(fd, DRM_IOCTL_I915_GEM_CREATE, &create) != 0) {
		wlr_log_errno(WLR_ERROR, "dmabuf check: GEM_CREATE failed");
		return 0;
	}
	// Written through a write-combined mapping: gen12 has no GEM_PWRITE, and
	// a WC write needs no cache flush to be seen by the GPU.
	struct drm_i915_gem_mmap_offset mo = {
		.handle = create.handle,
		.flags = I915_MMAP_OFFSET_WC,
	};
	if (drmIoctl(fd, DRM_IOCTL_I915_GEM_MMAP_OFFSET, &mo) != 0) {
		wlr_log_errno(WLR_ERROR, "dmabuf check: GEM_MMAP_OFFSET failed");
		gem_close(fd, create.handle);
		return 0;
	}
	uint32_t *map = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, mo.offset);
	if (map == MAP_FAILED) {
		wlr_log_errno(WLR_ERROR, "dmabuf check: mmap of the probe batch failed");
		gem_close(fd, create.handle);
		return 0;
	}
	map[0] = MI_BATCH_BUFFER_END;
	map[1] = 0;
	munmap(map, 4096);
	return create.handle;
}

// Submit the empty batch with `handles` in its object list. 0 or -errno.
static int probe_execbuf(struct cce_dmabuf_check *c, const uint32_t *handles, int n) {
	struct drm_i915_gem_exec_object2 objs[WLR_DMABUF_MAX_PLANES + 1];
	memset(objs, 0, sizeof(objs));
	for (int i = 0; i < n; i++) {
		objs[i].handle = handles[i];
		objs[i].flags = EXEC_OBJECT_SUPPORTS_48B_ADDRESS;
	}
	objs[n].handle = c->batch; // the batch is the last object
	objs[n].flags = EXEC_OBJECT_SUPPORTS_48B_ADDRESS;
	struct drm_i915_gem_execbuffer2 eb = {
		.buffers_ptr = (uint64_t)(uintptr_t)objs,
		.buffer_count = (uint32_t)n + 1,
		.batch_start_offset = 0,
		.batch_len = 8,
		.flags = I915_EXEC_RENDER | I915_EXEC_NO_RELOC,
	};
	if (drmIoctl(c->fd, DRM_IOCTL_I915_GEM_EXECBUFFER2, &eb) != 0) {
		return -errno;
	}
	return 0;
}

static bool check_dmabuf(struct wlr_dmabuf_attributes *attribs, void *data) {
	struct cce_dmabuf_check *c = data;
	uint32_t handles[WLR_DMABUF_MAX_PLANES];
	int n = 0;
	bool foreign = false;
	bool ok = true;

	for (int i = 0; i < attribs->n_planes; i++) {
		uint32_t handle = 0;
		// What wlroots' own check does: the device must be able to import it.
		if (drmPrimeFDToHandle(c->fd, attribs->fd[i], &handle) != 0) {
			wlr_log_errno(WLR_DEBUG, "dmabuf check: PRIME import of plane %d failed", i);
			ok = false;
			break;
		}
		bool seen = false;
		for (int j = 0; j < n; j++) {
			if (handles[j] == handle) {
				seen = true;
			}
		}
		if (!seen) {
			handles[n++] = handle;
		}
		char exp[32];
		exporter_name(attribs->fd[i], exp, sizeof(exp));
		if (strcmp(exp, "i915") != 0) {
			foreign = true;
		}
	}

	if (ok && c->i915 && foreign && c->batch != 0) {
		int err = probe_execbuf(c, handles, n);
		if (err != 0) {
			char exp[32];
			exporter_name(attribs->fd[0], exp, sizeof(exp));
			wlr_log(WLR_ERROR, "dmabuf check: refused a %dx%d buffer (format 0x%08x, modifier 0x%016llx) "
				"exported by '%s': this GPU cannot use its memory (execbuf: %s)",
				attribs->width, attribs->height, attribs->format,
				(unsigned long long)attribs->modifier, exp[0] ? exp : "?", strerror(-err));
			ok = false;
		}
	}

	for (int j = 0; j < n; j++) {
		gem_close(c->fd, handles[j]);
	}
	return ok;
}

// Install the check on `linux_dmabuf` for the renderer whose DRM fd is
// `renderer_fd`. Returns false (and leaves wlroots' default check in place)
// when the device cannot be opened.
bool cce_dmabuf_check_install(struct wlr_linux_dmabuf_v1 *linux_dmabuf, int renderer_fd) {
	if (linux_dmabuf == NULL || renderer_fd < 0 || check_state != NULL) {
		return false;
	}
	char *name = drmGetDeviceNameFromFd2(renderer_fd);
	if (name == NULL) {
		wlr_log(WLR_ERROR, "dmabuf check: no device name for the renderer fd");
		return false;
	}
	int fd = open(name, O_RDWR | O_CLOEXEC);
	if (fd < 0) {
		wlr_log_errno(WLR_ERROR, "dmabuf check: cannot open %s", name);
		free(name);
		return false;
	}
	struct cce_dmabuf_check *c = calloc(1, sizeof(*c));
	if (c == NULL) {
		close(fd);
		free(name);
		return false;
	}
	c->fd = fd;
	drmVersionPtr ver = drmGetVersion(fd);
	c->i915 = ver != NULL && strcmp(ver->name, "i915") == 0;
	if (ver != NULL) {
		drmFreeVersion(ver);
	}
	if (c->i915) {
		c->batch = create_batch(fd);
	}
	wlr_log(WLR_INFO, "dmabuf check: installed on %s (%s)", name,
		!c->i915 ? "import check only, not i915"
		: c->batch != 0 ? "foreign buffers probed with an empty batch"
		: "import check only, no probe batch");
	free(name);
	check_state = c;
	wlr_linux_dmabuf_v1_set_check_dmabuf_callback(linux_dmabuf, check_dmabuf, c);
	return true;
}
