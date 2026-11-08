// Executes the production policy. POSIX process tests are supplemental:
// they do not replace authenticated Sparkle XPC or GPUI integration tests.
#if defined(__APPLE__)
#define _DARWIN_C_SOURCE 1
#else
#define _POSIX_C_SOURCE 200809L
#endif
#include "SPUInstallationCommitState.h"
#include <assert.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static const uint8_t nonce[SPU_INSTALLATION_NONCE_LENGTH] = {1, 4, 9};

static void put(int fd, char value)
{
    assert(write(fd, &value, 1) == 1);
}

static char get(int fd)
{
    char value = 0;
    ssize_t result = read(fd, &value, 1);
    assert(result == 0 || result == 1);
    return value;
}

static void marker(const char *path, char value)
{
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    assert(fd >= 0);
    put(fd, value);
    assert(close(fd) == 0);
}

static char read_marker(const char *path)
{
    int fd = open(path, O_RDONLY);
    assert(fd >= 0);
    char value = get(fd);
    assert(close(fd) == 0);
    return value;
}

static void unit_policy(void)
{
    uint8_t wrong[sizeof(nonce)] = {7};
    SPUInstallationCommitState state = {0};
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 1));
    assert(!SPUCommitBeginFinalInstallation(&state));
    assert(SPUCommitPrepare(&state, nonce, 1));
    assert(!SPUCommitPrepare(&state, nonce, 1));
    assert(!SPUCommitAuthorize(&state, wrong, sizeof(wrong), 2));
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce) - 1, 2));
    assert(SPUCommitDisarm(&state)); // Lost XPC or CANCEL.
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2));
    SPUCommitHostTerminated(&state);
    assert(!SPUCommitBeginFinalInstallation(&state));

    state = (SPUInstallationCommitState){0};
    assert(SPUCommitPrepare(&state, nonce, 1));
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 301));
    assert(state.phase == SPUCommitDisarmed);
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2));

    state = (SPUInstallationCommitState){0};
    SPUCommitHostTerminated(&state); // Crash before preparation finishes.
    assert(!SPUCommitPrepare(&state, nonce, 1));
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2));
    assert(!SPUCommitBeginFinalInstallation(&state));

    state = (SPUInstallationCommitState){0};
    assert(SPUCommitPrepare(&state, wrong, 1));
    assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2)); // Session replay.
    assert(SPUCommitAuthorize(&state, wrong, sizeof(wrong), 2));
    assert(!SPUCommitAuthorize(&state, wrong, sizeof(wrong), 2));
    assert(!SPUCommitDisarm(&state));
    assert(!SPUCommitBeginFinalInstallation(&state));
    SPUCommitHostTerminated(&state);
    assert(SPUCommitBeginFinalInstallation(&state));
    assert(!SPUCommitBeginFinalInstallation(&state));
}

static void process_session(char mode)
{
    char root[] = "/tmp/sparkle-commit-test-XXXXXX";
    assert(mkdtemp(root) != NULL);
    char original[256], staging[256];
    assert(snprintf(original, sizeof(original), "%s/installed", root) > 0);
    assert(snprintf(staging, sizeof(staging), "%s/staged", root) > 0);
    marker(original, 'O');
    marker(staging, 'S');
    int requests[2], replies[2];
    assert(pipe(requests) == 0);
    assert(pipe(replies) == 0);
    pid_t host = fork();
    assert(host >= 0);
    if (host == 0) {
        close(requests[0]); close(replies[1]);
        if (mode == 'K') {
            raise(SIGKILL); // Host crash before COMMIT; no final user reply.
            _exit(99);
        }
        if (mode == 'E') {
            close(requests[1]); // Lost host connection before COMMIT.
            assert(get(replies[0]) == 'D');
            assert(read_marker(original) == 'O');
            _exit(0);
        }
        put(requests[1], mode);
        assert(get(replies[0]) == 'A');
        assert(read_marker(original) == 'O');
        if (mode == 'X') {
            assert(access(staging, F_OK) != 0); // ACK follows disarm + cleanup.
        } else {
            assert(access(staging, F_OK) == 0);
            put(requests[1], 'W'); // Host is still alive after COMMIT ACK.
            assert(get(replies[0]) == 'Q');
        }
        _exit(0);
    }
    close(requests[1]); close(replies[0]);
    SPUInstallationCommitState state = {0};
    assert(SPUCommitPrepare(&state, nonce, 1));
    char request = get(requests[0]);
    if (request == 'C') {
        assert(SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2));
        assert(!SPUCommitBeginFinalInstallation(&state));
        put(replies[1], 'A');
        assert(get(requests[0]) == 'W');
        int status = 0;
        assert(waitpid(host, &status, WNOHANG) == 0);
        assert(read_marker(original) == 'O');
        assert(!SPUCommitBeginFinalInstallation(&state));
        put(replies[1], 'Q');
    } else {
        assert(request == 'X' || request == 0);
        assert(SPUCommitDisarm(&state));
        assert(unlink(staging) == 0);
        if (request == 'X') {
            put(replies[1], 'A');
        } else if (mode == 'E') {
            put(replies[1], 'D');
        }
    }
    int status = 0;
    assert(waitpid(host, &status, 0) == host); // Real process wait, not a sleep.
    if (mode == 'K') {
        assert(WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL);
    } else {
        assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);
    }
    SPUCommitHostTerminated(&state);
    if (mode == 'C') {
        assert(SPUCommitBeginFinalInstallation(&state));
        marker(original, 'N');
        assert(!SPUCommitBeginFinalInstallation(&state));
        assert(unlink(staging) == 0);
        assert(read_marker(original) == 'N');
    } else {
        assert(!SPUCommitAuthorize(&state, nonce, sizeof(nonce), 2));
        assert(!SPUCommitBeginFinalInstallation(&state));
        assert(read_marker(original) == 'O');
    }
    assert(access(staging, F_OK) != 0);
    close(requests[0]); close(replies[1]);
    assert(unlink(original) == 0);
    assert(rmdir(root) == 0);
}

int main(void)
{
    unit_policy();
    process_session('X');
    process_session('K');
    process_session('E');
    process_session('C');
    puts("PASS: production commit policy; CANCEL ACK, host crash, lost connection, COMMIT then real wait/once");
    return 0;
}
