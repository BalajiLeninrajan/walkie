// Thin wrapper over transcribe.cpp so the Rust side never mirrors its structs.
//
// The parameter structs in transcribe.h grow between releases; keeping their
// layout on the C side means a submodule bump can't silently corrupt them.

#include "transcribe.h"

#include <stdlib.h>

struct walkie_engine {
    struct transcribe_session * session;
};

// Returns 0 on success. On failure *err points to a static status string.
int walkie_engine_open(const char * model, int threads, struct walkie_engine ** out, const char ** err) {
    struct transcribe_session_params sp;
    transcribe_session_params_init(&sp);
    sp.n_threads = threads;

    struct transcribe_session * session = NULL;
    transcribe_status           st      = transcribe_open(model, NULL, &sp, &session);
    if (st != TRANSCRIBE_OK) {
        *err = transcribe_status_string(st);
        return 1;
    }
    struct walkie_engine * e = malloc(sizeof *e);
    if (!e) {
        transcribe_close(session);
        *err = "out of memory";
        return 1;
    }
    e->session = session;
    *out       = e;
    return 0;
}

// Transcribes 16 kHz mono float PCM. On success *text stays valid until the
// next run or close.
int walkie_engine_run(struct walkie_engine * e, const float * pcm, int n, const char ** text, const char ** err) {
    struct transcribe_run_params rp;
    transcribe_run_params_init(&rp);
    rp.timestamps = TRANSCRIBE_TIMESTAMPS_NONE;

    transcribe_status st = transcribe_run(e->session, pcm, n, &rp);
    if (st != TRANSCRIBE_OK) {
        *err = transcribe_status_string(st);
        return 1;
    }
    *text = transcribe_full_text(e->session);
    return 0;
}

void walkie_engine_close(struct walkie_engine * e) {
    if (e) {
        transcribe_close(e->session);
        free(e);
    }
}
