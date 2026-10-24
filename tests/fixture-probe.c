#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/ptrace.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <signal.h>
#include <unistd.h>

#define REQUIRE(x) do { if (!(x)) { fprintf(stderr, "probe failed: %s (errno %d)\n", #x, errno); return 99; } } while (0)

int main(int argc, char **argv) {
    REQUIRE(argc >= 2);
    if (!strcmp(argv[1], "namespaces")) {
        const char *names[] = { "user", "mnt", "pid", "net", "ipc", "uts", "cgroup" };
        REQUIRE(argc == 9);
        for (int i = 0; i < 7; i++) {
            char path[128], target[128];
            snprintf(path, sizeof(path), "/proc/self/ns/%s", names[i]);
            ssize_t n = readlink(path, target, sizeof(target) - 1);
            REQUIRE(n > 0); target[n] = 0;
            REQUIRE(strcmp(target, argv[i + 2]) != 0);
        }
        puts("private namespaces");
        return 0;
    }
    if (!strcmp(argv[1], "echo")) {
        char chunk[4096];
        ssize_t n;
        while ((n = read(0, chunk, sizeof(chunk))) > 0) {
            for (int fd = 1; fd <= 2; fd++) {
                ssize_t offset = 0;
                while (offset < n) { ssize_t wrote = write(fd, chunk + offset, n - offset); REQUIRE(wrote > 0); offset += wrote; }
            }
        }
        REQUIRE(n == 0);
        return 0;
    }
    if (!strcmp(argv[1], "exit")) return 23;
    if (!strcmp(argv[1], "burst")) {
        char bytes[4096]; memset(bytes, 'b', sizeof(bytes));
        for (int i = 0; i < 512; i++) for (int fd = 1; fd <= 2; fd++) {
            size_t offset = 0;
            while (offset < sizeof(bytes)) { ssize_t n = write(fd, bytes + offset, sizeof(bytes) - offset); REQUIRE(n > 0); offset += n; }
        }
        return 0;
    }
    if (!strcmp(argv[1], "output")) {
        char bytes[4096]; memset(bytes, 'x', sizeof(bytes));
        for (;;) { if (write(1, bytes, sizeof(bytes)) < 0) return 0; }
    }
    if (!strcmp(argv[1], "block")) { for (;;) pause(); }
    if (!strcmp(argv[1], "descendant")) {
        REQUIRE(fork() >= 0);
        signal(SIGTERM, SIG_IGN);
        for (;;) pause();
    }
    if (!strcmp(argv[1], "memory")) {
        pid_t pid = fork();
        REQUIRE(pid >= 0);
        if (pid > 0) { int status; waitpid(pid, &status, 0); return 88; }
        for (;;) { volatile char *p = malloc(1024 * 1024); REQUIRE(p != NULL); for (int i = 0; i < 1024 * 1024; i += 4096) p[i] = 1; }
    }
    if (!strcmp(argv[1], "kill-group")) {
        pid_t pid = fork();
        REQUIRE(pid >= 0);
        if (pid == 0) { for (;;) pause(); }
        REQUIRE(kill(0, SIGKILL) == 0);
        int status;
        REQUIRE(waitpid(pid, &status, 0) == pid);
        REQUIRE(WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL);
        return 0; /* namespace PID 1 is protected from its own group signal */
    }
    if (!strcmp(argv[1], "pids")) {
        int children = 0;
        while (children < 300) {
            pid_t pid = fork();
            if (pid < 0) { REQUIRE(errno == EAGAIN); break; }
            if (pid == 0) { for (;;) pause(); }
            children++;
        }
        REQUIRE(children > 0 && children < 16);
        printf("bounded children %d\n", children);
        return 0;
    }
    if (!strcmp(argv[1], "write")) {
        REQUIRE(argc == 3);
        FILE *f = fopen("/home/user/value", "w");
        REQUIRE(f && fputs(argv[2], f) >= 0 && fclose(f) == 0);
        return 0;
    }
    if (!strcmp(argv[1], "read")) {
        FILE *f = fopen("/home/user/value", "r");
        if (!f) { REQUIRE(errno == ENOENT); puts("missing"); return 0; }
        int c; while ((c = fgetc(f)) != EOF) putchar(c);
        fclose(f);
        return 0;
    }
    if (!strcmp(argv[1], "symlink-data")) {
        REQUIRE(symlink("/home/alice/untouched", "/home/user/escape") == 0);
        REQUIRE(mkdir("/home/user/readonly", 0700) == 0);
        REQUIRE(chmod("/home/user/readonly", 0000) == 0);
        return 0;
    }
    if (!strcmp(argv[1], "baseline")) {
        for (int fd = 3; fd < 1024; fd++) REQUIRE(fcntl(fd, F_GETFD) == -1 && errno == EBADF);
        REQUIRE(getpid() == 1 && getppid() == 0);
        REQUIRE(open("/app/bin/fixture", O_WRONLY) == -1 && errno == EROFS);
        REQUIRE(open("/usr/payload", O_WRONLY) == -1 && errno == EROFS);
        REQUIRE(access("/home/alice/untouched", F_OK) == -1);
        REQUIRE(access("/app/recovery-escape/marker", F_OK) == -1);
        REQUIRE(access("/run/user/1000/flatpak-rs/management.sock", F_OK) == -1);
        REQUIRE(access("/dev/dri", F_OK) == -1);
        REQUIRE(access("/dev/snd", F_OK) == -1);
        REQUIRE(access("/dev/input", F_OK) == -1);
        REQUIRE(getenv("FLATPAK_RS_HOST_SECRET") == NULL);
        REQUIRE(!strcmp(getenv("HOME"), "/home/user"));
        REQUIRE(!strcmp(getenv("PATH"), "/app/bin:/usr/bin"));
        REQUIRE(!strcmp(getenv("XDG_DATA_HOME"), "/home/user/.local/share"));
        REQUIRE(!strcmp(getenv("XDG_CONFIG_HOME"), "/home/user/.config"));
        REQUIRE(!strcmp(getenv("XDG_CACHE_HOME"), "/home/user/.cache"));
        REQUIRE(getenv("HOST_XDG_DATA_HOME") == NULL);
        REQUIRE(socket(AF_INET, SOCK_STREAM, 0) == -1 && errno == EPERM);
        REQUIRE(unshare(CLONE_NEWUSER) == -1 && errno == EPERM);
        /* Reject unreviewed high clone bits, not only known namespace flags. */
        long cloned = syscall(SYS_clone, (1ULL << 32) | SIGCHLD, NULL, NULL, NULL, 0);
        if (cloned == 0) _exit(0);
        if (cloned > 0) waitpid(cloned, NULL, 0);
        REQUIRE(cloned == -1 && errno == EPERM);
        REQUIRE(mount("none", "/tmp", "tmpfs", 0, NULL) == -1 && errno == EPERM);
        REQUIRE(ptrace(PTRACE_TRACEME, 0, NULL, NULL) == -1 && errno == EPERM);
        REQUIRE(syscall(SYS_getpriority, 0, 0) == -1 && errno == EPERM);
        for (int capability = 0; capability <= 40; capability++) REQUIRE(prctl(PR_CAPBSET_READ, capability, 0, 0, 0) == 0);
        FILE *status = fopen("/proc/self/status", "r");
        REQUIRE(status != NULL);
        char buffer[8192];
        size_t n = fread(buffer, 1, sizeof(buffer) - 1, status);
        buffer[n] = 0;
        fclose(status);
        REQUIRE(strstr(buffer, "CapEff:\t0000000000000000") != NULL);
        REQUIRE(strstr(buffer, "NoNewPrivs:\t1") != NULL);
        REQUIRE(strstr(buffer, "Seccomp:\t2") != NULL);
        DIR *proc = opendir("/proc");
        REQUIRE(proc != NULL);
        struct dirent *entry;
        int processes = 0;
        while ((entry = readdir(proc))) if (entry->d_name[0] >= '0' && entry->d_name[0] <= '9') processes++;
        closedir(proc);
        REQUIRE(processes == 1);
        FILE *data = fopen("/home/user/value", "w");
        REQUIRE(data != NULL && fputs("saved", data) >= 0);
        REQUIRE(fclose(data) == 0);
        data = fopen("/tmp/private", "w");
        REQUIRE(data != NULL && fclose(data) == 0);
        puts("confined");
        fputs("fixture stderr\n", stderr);
        return 17;
    }
    return 98;
}
