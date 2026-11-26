// Minimal PipeWire client: one mono s16 capture stream and short one-shot
// playback streams for the beeps. All calls from Rust take the thread loop
// lock; the callbacks run on the loop thread.

#include <pipewire/pipewire.h>
#include <spa/param/audio/format-utils.h>
#include <spa/utils/result.h>

#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef void (*walkie_pcm_cb)(void * user, const int16_t * samples, uint32_t n);

#define MAX_PLAYS 4

struct play {
    struct walkie_pw *  pw;
    struct pw_stream * stream;
    struct spa_hook   listener;
    int16_t *         samples;
    uint32_t          n, pos;
    bool              draining, done;
};

struct walkie_pw {
    struct pw_thread_loop * loop;
    struct pw_context *     context;
    struct pw_core *        core;
    struct pw_stream *      capture;
    struct spa_hook         capture_listener;
    walkie_pcm_cb             cb;
    void *                  user;
    char                    error[256];
    struct play             plays[MAX_PLAYS];
};

static void set_error(struct walkie_pw * pw, const char * what, const char * detail) {
    snprintf(pw->error, sizeof pw->error, "%s: %s", what, detail ? detail : "unknown error");
}

// ---- capture ---------------------------------------------------------------

static void on_capture_process(void * data) {
    struct walkie_pw *   pw = data;
    struct pw_buffer * b  = pw_stream_dequeue_buffer(pw->capture);
    if (!b) {
        return;
    }
    struct spa_data * d = &b->buffer->datas[0];
    if (d->data && d->chunk) {
        uint32_t off  = SPA_MIN(d->chunk->offset, d->maxsize);
        uint32_t size = SPA_MIN(d->chunk->size, d->maxsize - off);
        pw->cb(pw->user, SPA_PTROFF(d->data, off, const int16_t), size / sizeof(int16_t));
    }
    pw_stream_queue_buffer(pw->capture, b);
}

static void on_capture_state(void * data, enum pw_stream_state old, enum pw_stream_state state, const char * error) {
    (void) old;
    if (state == PW_STREAM_STATE_ERROR) {
        set_error(data, "capture stream", error);
    }
}

static const struct pw_stream_events capture_events = {
    PW_VERSION_STREAM_EVENTS,
    .state_changed = on_capture_state,
    .process       = on_capture_process,
};

// ---- playback --------------------------------------------------------------

static void on_play_process(void * data) {
    struct play *      p = data;
    struct pw_buffer * b = pw_stream_dequeue_buffer(p->stream);
    if (!b) {
        return;
    }
    struct spa_data * d   = &b->buffer->datas[0];
    uint32_t          n   = 0;
    if (d->data) {
        uint32_t max = d->maxsize / sizeof(int16_t);
        if (b->requested && b->requested < max) {
            max = (uint32_t) b->requested;
        }
        n = SPA_MIN(p->n - p->pos, max);
        memcpy(d->data, p->samples + p->pos, n * sizeof(int16_t));
        p->pos += n;
    }
    d->chunk->offset = 0;
    d->chunk->stride = sizeof(int16_t);
    d->chunk->size   = n * sizeof(int16_t);
    pw_stream_queue_buffer(p->stream, b);
    if (p->pos >= p->n && !p->draining) {
        p->draining = true;
        pw_stream_flush(p->stream, true);
    }
}

static void on_play_drained(void * data) {
    struct play * p = data;
    p->done         = true;
    pw_thread_loop_signal(p->pw->loop, false);
}

static void on_play_state(void * data, enum pw_stream_state old, enum pw_stream_state state, const char * error) {
    (void) old;
    (void) error;
    struct play * p = data;
    if (state == PW_STREAM_STATE_ERROR || state == PW_STREAM_STATE_UNCONNECTED) {
        p->done = true;
        pw_thread_loop_signal(p->pw->loop, false);
    }
}

static const struct pw_stream_events play_events = {
    PW_VERSION_STREAM_EVENTS,
    .state_changed = on_play_state,
    .process       = on_play_process,
    .drained       = on_play_drained,
};

static void play_reap(struct play * p) {  // loop locked
    if (p->stream) {
        pw_stream_destroy(p->stream);
        p->stream = NULL;
    }
    free(p->samples);
    p->samples = NULL;
}

// ---- public API ------------------------------------------------------------

const char * walkie_pw_error(struct walkie_pw * pw) {
    return pw->error[0] ? pw->error : NULL;
}

// Connects to the PipeWire daemon. Returns NULL and fills err on failure.
struct walkie_pw * walkie_pw_new(char * err, size_t errlen) {
    pw_init(NULL, NULL);
    struct walkie_pw * pw = calloc(1, sizeof *pw);
    if (!pw) {
        snprintf(err, errlen, "out of memory");
        return NULL;
    }
    pw->loop = pw_thread_loop_new("walkie-pw", NULL);
    if (!pw->loop) {
        snprintf(err, errlen, "pw_thread_loop_new failed");
        free(pw);
        return NULL;
    }
    pw_thread_loop_lock(pw->loop);
    if (pw_thread_loop_start(pw->loop) < 0) {
        snprintf(err, errlen, "could not start the PipeWire thread");
        goto fail;
    }
    pw->context = pw_context_new(pw_thread_loop_get_loop(pw->loop), NULL, 0);
    if (!pw->context) {
        snprintf(err, errlen, "pw_context_new failed");
        goto fail;
    }
    pw->core = pw_context_connect(pw->context, NULL, 0);
    if (!pw->core) {
        snprintf(err, errlen, "could not connect to PipeWire: %s", strerror(errno));
        goto fail;
    }
    pw_thread_loop_unlock(pw->loop);
    return pw;
fail:
    pw_thread_loop_unlock(pw->loop);
    pw_thread_loop_stop(pw->loop);
    if (pw->context) {
        pw_context_destroy(pw->context);
    }
    pw_thread_loop_destroy(pw->loop);
    free(pw);
    return NULL;
}

static const struct spa_pod * mono_s16(struct spa_pod_builder * b, uint32_t rate) {
    struct spa_audio_info_raw info = SPA_AUDIO_INFO_RAW_INIT(.format = SPA_AUDIO_FORMAT_S16, .rate = rate, .channels = 1,
                                                             .position = { SPA_AUDIO_CHANNEL_MONO });
    return spa_format_audio_raw_build(b, SPA_PARAM_EnumFormat, &info);
}

// Starts recording 16 kHz mono s16 from the default source. cb runs on the
// PipeWire thread with each chunk. Returns 0 on success.
int walkie_pw_capture_start(struct walkie_pw * pw, walkie_pcm_cb cb, void * user) {
    pw_thread_loop_lock(pw->loop);
    pw->cb      = cb;
    pw->user    = user;
    pw->capture = pw_stream_new(pw->core, "walkie",
                                pw_properties_new(PW_KEY_MEDIA_TYPE, "Audio", PW_KEY_MEDIA_CATEGORY, "Capture",
                                                  PW_KEY_MEDIA_ROLE, "Communication", PW_KEY_APP_NAME, "walkie",
                                                  PW_KEY_NODE_LATENCY, "320/16000", NULL));
    int res = -1;
    if (pw->capture) {
        pw_stream_add_listener(pw->capture, &pw->capture_listener, &capture_events, pw);
        uint8_t                buf[1024];
        struct spa_pod_builder b         = SPA_POD_BUILDER_INIT(buf, sizeof buf);
        const struct spa_pod * params[1] = { mono_s16(&b, 16000) };
        res = pw_stream_connect(pw->capture, PW_DIRECTION_INPUT, PW_ID_ANY,
                                PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS, params, 1);
        if (res < 0) {
            set_error(pw, "capture connect", spa_strerror(res));
            pw_stream_destroy(pw->capture);  // don't keep a stream that points at the caller's data
            pw->capture = NULL;
        }
    } else {
        set_error(pw, "capture stream", strerror(errno));
    }
    pw_thread_loop_unlock(pw->loop);
    return res < 0 ? 1 : 0;
}

// Stops recording. After this returns the callback never runs again.
void walkie_pw_capture_stop(struct walkie_pw * pw) {
    pw_thread_loop_lock(pw->loop);
    if (pw->capture) {
        pw_stream_destroy(pw->capture);
        pw->capture = NULL;
    }
    pw_thread_loop_unlock(pw->loop);
}

// Starts playing mono s16 samples at `rate`. Returns at once; the samples are
// copied. Returns 0 on success.
int walkie_pw_play(struct walkie_pw * pw, const int16_t * samples, uint32_t n, uint32_t rate) {
    pw_thread_loop_lock(pw->loop);
    struct play * p = NULL;
    for (int i = 0; i < MAX_PLAYS; i++) {
        if (pw->plays[i].done) {
            play_reap(&pw->plays[i]);
            memset(&pw->plays[i], 0, sizeof pw->plays[i]);
        }
        if (!p && !pw->plays[i].stream) {
            p = &pw->plays[i];
        }
    }
    int res = -1;
    if (p && (p->samples = malloc(n * sizeof(int16_t)))) {
        memcpy(p->samples, samples, n * sizeof(int16_t));
        p->pw     = pw;
        p->n      = n;
        p->stream = pw_stream_new(pw->core, "walkie-beep",
                                  pw_properties_new(PW_KEY_MEDIA_TYPE, "Audio", PW_KEY_MEDIA_CATEGORY, "Playback",
                                                    PW_KEY_MEDIA_ROLE, "Notification", PW_KEY_APP_NAME, "walkie",
                                                    NULL));
        if (p->stream) {
            pw_stream_add_listener(p->stream, &p->listener, &play_events, p);
            uint8_t                buf[1024];
            struct spa_pod_builder b         = SPA_POD_BUILDER_INIT(buf, sizeof buf);
            const struct spa_pod * params[1] = { mono_s16(&b, rate) };
            res = pw_stream_connect(p->stream, PW_DIRECTION_OUTPUT, PW_ID_ANY,
                                    PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS, params, 1);
        }
        if (res < 0) {
            play_reap(p);
            memset(p, 0, sizeof *p);
        }
    }
    pw_thread_loop_unlock(pw->loop);
    return res < 0 ? 1 : 0;
}

// Waits until every beep has finished, up to timeout_ms.
void walkie_pw_wait_playback(struct walkie_pw * pw, int timeout_ms) {
    pw_thread_loop_lock(pw->loop);
    struct timespec deadline;
    pw_thread_loop_get_time(pw->loop, &deadline, (int64_t) timeout_ms * SPA_NSEC_PER_MSEC);
    for (;;) {
        bool busy = false;
        for (int i = 0; i < MAX_PLAYS; i++) {
            busy |= pw->plays[i].stream && !pw->plays[i].done;
        }
        if (!busy || pw_thread_loop_timed_wait_full(pw->loop, &deadline) != 0) {
            break;
        }
    }
    pw_thread_loop_unlock(pw->loop);
}

void walkie_pw_free(struct walkie_pw * pw) {
    if (!pw) {
        return;
    }
    pw_thread_loop_lock(pw->loop);
    if (pw->capture) {
        pw_stream_destroy(pw->capture);
    }
    for (int i = 0; i < MAX_PLAYS; i++) {
        play_reap(&pw->plays[i]);
    }
    pw_core_disconnect(pw->core);
    pw_thread_loop_unlock(pw->loop);
    pw_thread_loop_stop(pw->loop);
    pw_context_destroy(pw->context);
    pw_thread_loop_destroy(pw->loop);
    free(pw);
}
